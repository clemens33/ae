//! The two SHAPES, black-box: what an INSTALLED `ae` refuses, what it ignores,
//! and what a CHECKOUT build honours instead.
//!
//! The subject has to be a real process, because the fact under test is
//! `current_exe()` — where this binary SITS. A library test can only be told
//! the answer; a copy of the product binary planted in a fixture version
//! directory IS the answer, and it is the only way to reach the installed arm
//! at all.
//!
//! What is planted is exactly what `install` publishes (slice Z3): a version
//! directory named for the crate version holding `ae-core`, `install` and a
//! two-line `SHA256SUMS`, under `<HOME>/.ae/versions/`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};

/// A fixture install: a HOME with one published version directory under it.
struct Install {
    scratch: PathBuf,
    home: PathBuf,
    version_dir: PathBuf,
}

impl Install {
    /// Plant the three members `install` publishes, with a well-formed manifest.
    fn plant(tag: &str) -> Self {
        let scratch = PathBuf::from(format!("/tmp/aeshape.{}.{tag}", std::process::id()));
        let _ = remove(&scratch);
        let home = scratch.join("home");
        let version_dir = home.join(".ae").join("versions").join(ae::VERSION);
        assert!(
            std::fs::create_dir_all(&version_dir).is_ok(),
            "a version directory"
        );
        let rig = Self {
            scratch,
            home,
            version_dir,
        };
        assert!(
            std::fs::copy(env!("CARGO_BIN_EXE_ae"), rig.core()).is_ok(),
            "the core member is a copy of the product binary"
        );
        rig.write_installer("#!/bin/sh\necho \"installer ran: $AE_VERSION\"\n");
        rig.write_manifest(&format!("{0}  ae-core\n{0}  install\n", "0".repeat(64)));
        rig
    }

    fn core(&self) -> PathBuf {
        self.version_dir.join("ae-core")
    }

    fn manifest(&self) -> PathBuf {
        self.version_dir.join("SHA256SUMS")
    }

    fn write_manifest(&self, text: &str) {
        let path = self.manifest();
        let _ = std::fs::remove_file(&path);
        assert!(std::fs::write(&path, text).is_ok(), "a manifest");
    }

    fn write_installer(&self, script: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.version_dir.join("install");
        let _ = std::fs::remove_file(&path);
        assert!(std::fs::write(&path, script).is_ok(), "an installer");
        assert!(
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o555)).is_ok(),
            "0555, as install publishes it"
        );
    }

    /// A directory that stands in for a session, with one helper LINK in it.
    ///
    /// A helper is a symlink to the core and its `argv[0]` dirname IS the
    /// session — which is the whole reason a helper must pay the install gate:
    /// it is another way to reach this same binary.
    fn helper(&self, name: &str) -> PathBuf {
        let dir = self.scratch.join("sess");
        let _ = std::fs::create_dir_all(&dir);
        let link = dir.join(name);
        let _ = std::fs::remove_file(&link);
        assert!(
            std::os::unix::fs::symlink(self.core(), &link).is_ok(),
            "a helper link"
        );
        link
    }

    /// Run the planted core AS the installed `ae`, with `HOME` pointing at the
    /// fixture and `extra` on top.
    fn run(&self, extra: &[(&str, &str)], argv: &[&str]) -> (Option<i32>, String, String) {
        self.run_as(&self.core(), extra, argv)
    }

    /// The same, invoked through `program` — a helper link, or the core itself.
    fn run_as(
        &self,
        program: &Path,
        extra: &[(&str, &str)],
        argv: &[&str],
    ) -> (Option<i32>, String, String) {
        #[allow(
            clippy::disallowed_types,
            reason = "the black-box door: an INSTALLED shape is a process whose current_exe() sits in a version directory, which only running that file produces"
        )]
        let mut command = std::process::Command::new(program);
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("AE_HOME")
            .env_remove("CONFIG_FILE")
            .env_remove("AE_TMUX_SERVER")
            .env_remove("AE_TMUX_SERVER_KIND")
            .env("HOME", &self.home)
            .env("TMUX_TMPDIR", &self.scratch)
            .current_dir(&self.scratch);
        for (name, value) in extra {
            command.env(name, value);
        }
        let out = command
            .args(argv)
            .output()
            .unwrap_or_else(|why| panic!("the planted core should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for Install {
    fn drop(&mut self) {
        let _ = remove(&self.scratch);
    }
}

/// Remove a tree whose members may be 0555 — the mode `install` publishes.
fn remove(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o755));
            if entry.path().is_dir() {
                let _ = remove(&entry.path());
            }
        }
    }
    std::fs::remove_dir_all(path)
}

#[test]
fn a_well_formed_version_directory_runs_and_pins_its_own_home() {
    let rig = Install::plant("valid");
    let (code, stdout, stderr) = rig.run(&[], &["doctor"]);
    assert!(code.is_some(), "the planted core ran");
    let report = format!("{stdout}{stderr}");
    assert!(
        report.contains(&rig.home.join(".ae").join("sessions").display().to_string()),
        "an installed ae keeps its state under $HOME/.ae: {report}"
    );
    assert!(
        !report.contains("ignoring inherited"),
        "nothing was set, so nothing is named: {report}"
    );
}

#[test]
fn an_installed_ae_ignores_the_home_and_server_doors_and_says_which() {
    let rig = Install::plant("ignore");
    let (_, stdout, stderr) = rig.run(
        &[
            ("AE_HOME", "/evil/ae"),
            ("CONFIG_FILE", "/evil/config"),
            ("AE_TMUX_SERVER_KIND", "name"),
            ("AE_TMUX_SERVER", "someone-elses"),
        ],
        &["doctor"],
    );
    assert!(
        stderr.contains("ae: ignoring inherited"),
        "one aggregated notice: {stderr}"
    );
    for named in [
        "AE_HOME=/evil/ae",
        "CONFIG_FILE=/evil/config",
        "AE_TMUX_SERVER=someone-elses",
        "AE_TMUX_SERVER_KIND=name",
    ] {
        assert!(stderr.contains(named), "{named} unnamed in: {stderr}");
    }
    assert_eq!(
        stderr.matches("ignoring inherited").count(),
        1,
        "ONE notice, not one per variable: {stderr}"
    );
    let report = format!("{stdout}{stderr}");
    assert!(
        !report.contains("/evil/ae/sessions"),
        "the ignored home must not reach the state root: {report}"
    );
    assert!(
        report.contains(&rig.home.join(".ae").join("sessions").display().to_string()),
        "the pinned home did: {report}"
    );
}

#[test]
fn a_home_equal_to_the_default_is_not_worth_a_notice() {
    let rig = Install::plant("samehome");
    let same = rig.home.join(".ae");
    let (_, _, stderr) = rig.run(
        &[
            ("AE_HOME", &same.display().to_string()),
            ("CONFIG_FILE", &same.join("config").display().to_string()),
        ],
        &["doctor"],
    );
    assert!(
        !stderr.contains("ignoring inherited"),
        "a value that changes nothing is not a deviation: {stderr}"
    );
}

#[test]
fn a_tampered_manifest_refuses_before_any_effect() {
    let rig = Install::plant("tampered");
    for (text, why) in [
        ("nonsense\n", "not the format at all"),
        (
            &format!("{}  ae-core\n", "0".repeat(64)),
            "one member, not two",
        ),
        (
            &format!("{}  ae-core\n{0}  install\n", "0".repeat(63)),
            "a short digest",
        ),
        (
            &format!("{0}  ae-core\n{0}  install", "0".repeat(64)),
            "no trailing newline",
        ),
    ] {
        rig.write_manifest(text);
        let (code, stdout, stderr) = rig.run(&[], &["list"]);
        assert_eq!(code, Some(2), "{why}: {stdout}{stderr}");
        assert!(stdout.is_empty(), "{why}: a refusal must not reach stdout");
        assert!(stderr.contains("SHA256SUMS"), "{why}: {stderr}");
        assert!(stderr.contains("ae upgrade"), "{why}: {stderr}");
        assert!(
            !rig.home.join(".ae").join("sessions").exists(),
            "{why}: a refused invocation created state"
        );
    }
}

#[test]
fn a_missing_or_linked_member_refuses_too() {
    let rig = Install::plant("members");
    let installer = rig.version_dir.join("install");
    assert!(std::fs::remove_file(&installer).is_ok(), "remove a member");
    let (code, _, stderr) = rig.run(&[], &["list"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("missing install"), "{stderr}");

    // A LINK is not a member, and the classification is an lstat so it never
    // reads through to whatever the link points at.
    assert!(
        std::os::unix::fs::symlink("/bin/sh", &installer).is_ok(),
        "a link where a member belongs"
    );
    let (code, _, stderr) = rig.run(&[], &["list"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("not a regular file"), "{stderr}");
}

#[test]
fn a_version_directory_named_for_another_version_refuses() {
    let rig = Install::plant("wrongver");
    let wrong = rig.home.join(".ae").join("versions").join("1999.1.1");
    assert!(std::fs::create_dir_all(&wrong).is_ok(), "a second version");
    assert!(
        std::fs::copy(rig.core(), wrong.join("ae-core")).is_ok(),
        "the same core, published under a name it does not report"
    );
    assert!(
        std::fs::copy(rig.version_dir.join("install"), wrong.join("install")).is_ok(),
        "and its installer"
    );
    assert!(
        std::fs::copy(rig.manifest(), wrong.join("SHA256SUMS")).is_ok(),
        "and its manifest"
    );
    #[allow(
        clippy::disallowed_types,
        reason = "the black-box door: the shape under test is where the binary SITS"
    )]
    let out = std::process::Command::new(wrong.join("ae-core"))
        .env("HOME", &rig.home)
        .env_remove("AE_HOME")
        .arg("list")
        .output()
        .unwrap_or_else(|why| panic!("the planted core should run: {why}"));
    assert_eq!(out.status.code(), Some(2), "{:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("1999.1.1"), "{stderr}");
    assert!(stderr.contains(ae::VERSION), "{stderr}");
}

#[test]
fn version_and_upgrade_answer_on_a_broken_install() {
    // The whole reason both sit ahead of the gate: `version` DIAGNOSES a broken
    // install and `upgrade` REPAIRS one, so neither may depend on it.
    let rig = Install::plant("broken");
    rig.write_manifest("nonsense\n");

    let (code, stdout, _) = rig.run(&[], &["version"]);
    assert_eq!(code, Some(0), "version answers anyway");
    assert_eq!(stdout, format!("{}\n", ae::version_line()));

    let (code, stdout, stderr) = rig.run(&[("AE_VERSION", "2026.9.9")], &["upgrade"]);
    assert_eq!(code, Some(0), "the installer ran: {stdout}{stderr}");
    assert!(
        stdout.contains("ae upgrade: running"),
        "it names what it becomes: {stdout}"
    );
    assert!(
        stdout.contains("installer ran: 2026.9.9"),
        "AE_VERSION crossed the exec as the pin: {stdout}"
    );
}

#[test]
fn upgrade_takes_no_argument_and_refuses_before_it_runs_anything() {
    let rig = Install::plant("upgradeargv");
    let (code, stdout, stderr) = rig.run(&[], &["upgrade", "2026.9.9"]);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(stdout.is_empty(), "nothing ran: {stdout}");
    assert!(stderr.contains("AE_VERSION"), "{stderr}");
    assert!(!stderr.contains("installer ran"), "{stderr}");
}

#[test]
fn upgrade_refuses_an_installer_that_is_not_an_immutable_member() {
    let rig = Install::plant("upgradelink");
    let installer = rig.version_dir.join("install");
    assert!(
        std::fs::remove_file(&installer).is_ok(),
        "remove the member"
    );
    assert!(
        std::os::unix::fs::symlink("/bin/echo", &installer).is_ok(),
        "a link to something mutable and external"
    );
    let (code, stdout, stderr) = rig.run(&[], &["upgrade"]);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(stderr.contains("no installer beside"), "{stderr}");
}

#[test]
fn doctor_warns_when_the_published_core_is_writable_and_not_when_it_is_not() {
    use std::os::unix::fs::PermissionsExt as _;
    let rig = Install::plant("writable");

    // As `install` publishes it.
    assert!(
        std::fs::set_permissions(rig.core(), std::fs::Permissions::from_mode(0o555)).is_ok(),
        "0555"
    );
    let (_, stdout, stderr) = rig.run(&[], &["doctor"]);
    let report = format!("{stdout}{stderr}");
    assert!(
        !report.contains("is writable"),
        "a read-only core is not a warning: {report}"
    );

    // And after somebody chmods it — the state a helper link writes through to.
    assert!(
        std::fs::set_permissions(rig.core(), std::fs::Permissions::from_mode(0o755)).is_ok(),
        "0755"
    );
    let (_, stdout, stderr) = rig.run(&[], &["doctor"]);
    let report = format!("{stdout}{stderr}");
    assert!(report.contains("is writable"), "{report}");
    assert!(
        report.contains("WARN"),
        "a warning, never a refusal — doctor has to RUN to say it: {report}"
    );
}

/// **B1.** The gate is not a property of the PUBLIC words — it is a property of
/// the invocation, and the core's own `_` namespace is the most effectful part
/// of it. `_shims-render` on a core nobody can vouch for published 21 helper
/// links and answered 0.
#[test]
fn an_internal_entry_pays_the_install_gate_and_publishes_nothing() {
    let rig = Install::plant("internal");
    rig.write_manifest("nonsense\n");
    let session = rig.scratch.join("render");
    assert!(std::fs::create_dir_all(&session).is_ok(), "a session dir");

    let (code, stdout, stderr) = rig.run(&[], &["_shims-render", &session.display().to_string()]);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(stderr.contains("SHA256SUMS"), "{stderr}");
    assert!(stderr.contains("ae upgrade"), "{stderr}");
    let published = std::fs::read_dir(&session)
        .map(std::iter::Iterator::count)
        .unwrap_or_default();
    assert_eq!(published, 0, "a refused render published {published} links");
}

/// **B1, the other half.** Every session helper is a link to this binary, so a
/// helper that skipped the gate was 21 routes around it per session.
#[test]
fn a_session_helper_pays_the_install_gate_too() {
    let rig = Install::plant("helpergate");
    rig.write_manifest("nonsense\n");
    let send = rig.helper("send");

    let (code, stdout, stderr) = rig.run_as(&send, &[], &["someone", "hello"]);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(
        stderr.contains("SHA256SUMS"),
        "a helper must refuse for the SAME reason the public word does: {stderr}"
    );
}

/// **B2.** A published core run against a foreign `$HOME` used to classify as a
/// CHECKOUT, which honours `AE_HOME` — so an install could be pointed at
/// somebody else's state root and would build sessions there.
#[test]
fn a_published_core_refuses_a_foreign_home_instead_of_adopting_it() {
    let rig = Install::plant("foreignhome");
    let fake = rig.scratch.join("fakehome");
    let foreign = rig.scratch.join("foreign-ae");
    assert!(std::fs::create_dir_all(&fake).is_ok(), "a second home");

    let (code, stdout, stderr) = rig.run(
        &[
            ("HOME", &fake.display().to_string()),
            ("AE_HOME", &foreign.display().to_string()),
        ],
        &["doctor"],
    );
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(
        !foreign.join("sessions").exists(),
        "a refused invocation built state under the foreign root"
    );
    // BOTH roots are named: which of the two is the mistake is the caller's to
    // know, and the published one is the resolved spelling this core answers
    // `current_exe()` with.
    let published = std::fs::canonicalize(&rig.home).unwrap_or_else(|_| rig.home.clone());
    assert!(
        stderr.contains(&published.join(".ae").display().to_string()),
        "the published root is unnamed: {stderr}"
    );
    assert!(
        stderr.contains(&fake.display().to_string()),
        "the inherited HOME is unnamed: {stderr}"
    );
}
