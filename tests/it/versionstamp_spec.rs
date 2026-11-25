//! Lead rulings (versionstamp): once per tool start, `_run` asks the seat's own
//! harness for `--version` through the process door, with a short deadline, and
//! records the answer as `version.<slot>`; a wrapper, a hang, a failure or a
//! garbled answer records `unknown <reason>`, never a guess. Real `ae _run`
//! processes run against inert shell fixtures that stand in for the vendor
//! binaries: the fixture decides what `--version` prints, ae decides what it
//! records.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures read and write only process-owned scratch paths"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::time::Duration;

/// The fixture script's body for `--version`; every other call just exits.
struct Rig(super::cli::OwnedScratch);

impl Rig {
    fn new(tag: &str) -> Self {
        let rig = Self(super::cli::OwnedScratch::root("versionstamp", tag));
        for dir in ["home", "state/sessions/fixture", "project", "bin"] {
            fs::create_dir_all(rig.0.join(dir)).expect("fixture directory");
        }
        rig
    }

    fn session(&self) -> PathBuf {
        self.0.join("state/sessions/fixture")
    }

    fn log(&self) -> PathBuf {
        self.0.join("calls.log")
    }

    /// An executable stand-in named `name`: it logs its argv, answers
    /// `--version` with `answer`, and exits 0 for anything else.
    fn tool(&self, name: &str, answer: &str) -> PathBuf {
        let path = self.0.join("bin").join(name);
        let _ = fs::remove_file(&path);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\nif [ \"$1\" = \"--version\" ]; then\n{answer}\nfi\nexit 0\n",
                self.log().display()
            ),
        )
        .expect("fixture tool");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("executable");
        path
    }

    /// One seat on `tool` with the probe switched on, as a profile command, a
    /// global config and a meta.
    fn seat(&self, tool: &std::path::Path) {
        self.seat_with(tool, Some("on"));
    }

    /// [`Rig::seat`], with `[workspace] version_probe` set to `probe` (absent
    /// when `None`).
    fn seat_with(&self, tool: &std::path::Path, probe: Option<&str>) {
        let probe = probe.map_or_else(String::new, |value| format!("version_probe = {value}\n"));
        fs::write(
            self.0.join("config"),
            format!(
                "[profiles]\np = \"{}\"\n[roster]\nlead = p\n[workspace]\n{probe}main = lead\nworkers = \n",
                tool.display()
            ),
        )
        .expect("fixture profile");
        fs::write(
            self.session().join("meta"),
            format!(
                "schema=2\nmode=local\nsession=fixture\norigin={}\nwork_dir={}\nconfig={}\nseat.main=lead\nprofile.main=p\nagent_bin.main={}\nlaunch_id.main=versionstamp-token\n",
                self.0.join("project").display(),
                self.0.join("project").display(),
                self.0.join("config").display(),
                tool.file_name().and_then(|name| name.to_str()).unwrap_or("-"),
            ),
        )
        .expect("fixture seat");
    }

    fn start(&self) -> Output {
        let mut cmd = super::cli::ae();
        cmd.env("HOME", self.0.join("home"))
            .env("AE_HOME", self.0.join("state"))
            // The one file is the seat's project config AND the global config the
            // probe switch is read from (`[workspace] version_probe`).
            .env("CONFIG_FILE", self.0.join("config"))
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .current_dir(self.0.join("project"))
            .arg("_run")
            .arg(self.session())
            .arg("main");
        let child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("real ae child");
        let out = super::cli::bounded(child, Duration::from_secs(30))
            .expect("ae must finish and the owned child must be reaped");
        assert!(out.status.success(), "{out:?}");
        out
    }

    fn meta(&self) -> String {
        fs::read_to_string(self.session().join("meta")).expect("meta")
    }

    /// Every `version.main` row of the meta, in order.
    fn rows(&self) -> Vec<String> {
        self.meta()
            .lines()
            .filter(|line| line.starts_with("version.main"))
            .map(str::to_owned)
            .collect()
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[test]
fn a_start_records_the_version_its_harness_prints() {
    let rig = Rig::new("prints");
    rig.seat(&rig.tool("claude", "echo '2.1.999 (Claude Code)'"));
    rig.start();
    assert_eq!(rig.rows(), ["version.main=2.1.999"], "{}", rig.meta());
    assert_eq!(
        rig.calls()
            .iter()
            .filter(|call| *call == "--version")
            .count(),
        1,
        "one --version call per start: {:?}",
        rig.calls()
    );
}

#[test]
fn the_row_reads_a_tool_that_names_itself_in_front_of_the_version() {
    let rig = Rig::new("named");
    rig.seat(&rig.tool("codex", "echo 'codex-cli 0.199.1'"));
    rig.start();
    assert_eq!(rig.rows(), ["version.main=0.199.1"], "{}", rig.meta());
}

#[test]
fn a_hung_harness_records_a_timeout_and_the_start_goes_on() {
    let rig = Rig::new("hangs");
    rig.seat(&rig.tool("claude", "exec sleep 120"));
    let started = std::time::Instant::now();
    rig.start();
    assert_eq!(
        rig.rows(),
        ["version.main=unknown timeout"],
        "{}",
        rig.meta()
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the probe stops at its own short deadline, not the test's"
    );
    assert!(
        rig.calls()
            .iter()
            .any(|call| !call.starts_with("--version")),
        "the start itself still ran the harness: {:?}",
        rig.calls()
    );
}

#[test]
fn a_garbled_answer_records_unparseable() {
    let rig = Rig::new("garbled");
    rig.seat(&rig.tool("claude", "echo 'this is not a release'"));
    rig.start();
    assert_eq!(
        rig.rows(),
        ["version.main=unknown unparseable"],
        "{}",
        rig.meta()
    );
}

#[test]
fn a_failing_version_call_records_failed_even_when_it_printed_a_version() {
    let rig = Rig::new("fails");
    rig.seat(&rig.tool("claude", "echo '2.1.999'\nexit 3"));
    rig.start();
    assert_eq!(
        rig.rows(),
        ["version.main=unknown failed"],
        "{}",
        rig.meta()
    );
}

#[test]
fn an_answer_past_the_cap_records_oversize() {
    let rig = Rig::new("oversize");
    rig.seat(&rig.tool("claude", "head -c 20000 /dev/zero | tr '\\0' 'a'"));
    rig.start();
    assert_eq!(
        rig.rows(),
        ["version.main=unknown oversize"],
        "{}",
        rig.meta()
    );
}

#[test]
fn a_wrapper_that_hides_the_binary_is_never_asked_and_records_unclassified() {
    let rig = Rig::new("wrapper");
    // `codex-mic` is not a binary ae classifies: whatever it prints, ae has no
    // rule that says it is a harness, so it is neither run nor believed.
    rig.seat(&rig.tool("codex-mic", "echo 'codex-cli 0.199.1'"));
    rig.start();
    assert_eq!(
        rig.rows(),
        ["version.main=unknown unclassified"],
        "{}",
        rig.meta()
    );
    assert!(
        rig.calls().iter().all(|call| call != "--version"),
        "an unclassified wrapper was asked for --version: {:?}",
        rig.calls()
    );
}

#[test]
fn each_start_replaces_the_one_row_and_never_adds_a_second() {
    let rig = Rig::new("replaces");
    let tool = rig.tool("claude", "echo '2.1.100'");
    rig.seat(&tool);
    rig.start();
    assert_eq!(rig.rows(), ["version.main=2.1.100"]);
    rig.tool("claude", "echo '2.1.101'");
    rig.start();
    assert_eq!(rig.rows(), ["version.main=2.1.101"], "{}", rig.meta());
}

#[test]
fn without_the_key_no_version_call_is_made_and_no_row_is_written() {
    let rig = Rig::new("keyless");
    rig.seat_with(&rig.tool("claude", "echo '2.1.999 (Claude Code)'"), None);
    rig.start();
    assert_eq!(rig.rows(), Vec::<String>::new(), "{}", rig.meta());
    assert!(
        rig.calls().iter().all(|call| call != "--version"),
        "the default asked the harness anyway: {:?}",
        rig.calls()
    );
    assert!(
        !rig.calls().is_empty(),
        "the start itself still ran the harness"
    );
}

#[test]
fn only_an_exact_on_asks() {
    for value in ["off", "ON", "true", "1", "yes", ""] {
        let rig = Rig::new("not-on");
        rig.seat_with(&rig.tool("claude", "echo '2.1.999'"), Some(value));
        rig.start();
        assert_eq!(
            rig.rows(),
            Vec::<String>::new(),
            "{value:?}: {}",
            rig.meta()
        );
        assert!(
            rig.calls().iter().all(|call| call != "--version"),
            "version_probe = {value:?} asked: {:?}",
            rig.calls()
        );
    }
}

#[test]
fn turning_the_probe_off_takes_back_the_row_an_earlier_start_wrote() {
    let rig = Rig::new("takeback");
    let tool = rig.tool("claude", "echo '2.1.100'");
    rig.seat(&tool);
    rig.start();
    assert_eq!(rig.rows(), ["version.main=2.1.100"]);
    rig.seat_with(&tool, None);
    // `seat_with` rewrote the meta; put the earlier start's row back.
    let meta = format!("{}version.main=2.1.100\n", rig.meta());
    fs::write(rig.session().join("meta"), meta).expect("earlier row");
    rig.start();
    assert_eq!(rig.rows(), Vec::<String>::new(), "{}", rig.meta());
}
