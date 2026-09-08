//! Black-box contract for `ae init`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real config paths; the product boundary remains in init.rs"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use super::cli::{OwnedScratch, ae};

struct Rig {
    scratch: OwnedScratch,
    config: PathBuf,
    bin: PathBuf,
}

impl Rig {
    fn new(tag: &str, tools: &[&str]) -> Self {
        let mut scratch = OwnedScratch::existing(PathBuf::from(format!(
            "/tmp/aeinit.{}.{tag}",
            std::process::id()
        )));
        scratch.add_tmux_server(scratch.join("tmux.sock"));
        let config = scratch.join("selected").join("config");
        let bin = scratch.join("bin");
        assert!(std::fs::create_dir_all(&bin).is_ok(), "fake bin");
        for tool in tools {
            let path = bin.join(tool);
            assert!(
                std::fs::write(&path, b"not executed\n").is_ok(),
                "fake {tool}"
            );
            assert!(
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).is_ok(),
                "executable {tool}"
            );
        }
        Self {
            scratch,
            config,
            bin,
        }
    }

    fn run(&self, args: &[&str]) -> (Option<i32>, String, String) {
        self.run_with_server(args, false)
    }

    fn run_with_server(
        &self,
        args: &[&str],
        private_server: bool,
    ) -> (Option<i32>, String, String) {
        let mut command = ae();
        command
            .env("HOME", self.scratch.join("home"))
            .env("AE_HOME", self.scratch.join("state"))
            .env("CONFIG_FILE", &self.config)
            .env("PATH", &self.bin)
            .args(args);
        if private_server {
            command
                .env("TMUX_TMPDIR", &self.scratch)
                .env("AE_TMUX_SERVER_KIND", "socket")
                .env("AE_TMUX_SERVER", self.scratch.join("tmux.sock"));
        }
        let out = command
            .output()
            .unwrap_or_else(|why| panic!("ae init runs: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn install_launch_fakes(&self) -> PathBuf {
        let marker = self.scratch.join("agents-launched");
        for tool in ["claude", "codex"] {
            let body = format!(
                "#!/usr/bin/perl\nuse strict;\nuse warnings;\nsystem(\"stty raw -echo 2>/dev/null\");\nbinmode(STDIN, ':raw');\nbinmode(STDOUT, ':raw');\n$| = 1;\nopen(my $marker, '>>', '{}') or die; print $marker \"{}\\n\"; close($marker);\nif ($0 =~ /codex$/ && $ENV{{AE_HOME}}) {{\n    if (opendir(my $sessions, \"$ENV{{AE_HOME}}/sessions\")) {{\n        for my $name (readdir($sessions)) {{\n            next if $name =~ /^\\./;\n            my $sid = \"$ENV{{AE_HOME}}/sessions/$name/codex.worker.0.sid\";\n            if (open(my $file, '>', $sid)) {{ print $file \"0199c0de-1234-4890-abcd-ef0123456789\\n\"; close($file); last; }}\n        }}\n        closedir($sessions);\n    }}\n}}\nprint \"\\e[?2004h\";\nprint \"\\e[H\\e[2J\";\nprint \"\\e[1m\\xe2\\x9d\\xaf\\e[0m\\xc2\\xa0\\r\\n\";\nprint ((\"\\xe2\\x94\\x80\" x 400), \"\\r\\n\");\nsleep 600;\n",
                marker.display(),
                tool,
            );
            let path = self.bin.join(tool);
            assert!(std::fs::write(&path, body).is_ok(), "launch fake {tool}");
            assert!(
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).is_ok(),
                "executable launch fake {tool}"
            );
        }
        let Some(tmux) = ae::doctor::resolve_on_path("tmux") else {
            panic!("tmux on test PATH");
        };
        assert!(std::os::unix::fs::symlink(tmux, self.bin.join("tmux")).is_ok());
        marker
    }
}

#[test]
fn init_discovers_claude_and_codex_then_writes_the_selected_checkout_config() {
    let rig = Rig::new("write", &["claude", "codex"]);
    let (code, stdout, stderr) = rig.run(&["init", "--yes"]);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");
    assert!(
        stdout.contains(&format!(
            "claude    executable found: {}",
            rig.bin.join("claude").display()
        )),
        "{stdout}"
    );
    assert!(stdout.contains("gemini    not on PATH"), "{stdout}");
    assert!(stdout.contains(&format!("Wrote config to {}", rig.config.display())));
    assert!(stderr.is_empty(), "{stderr}");

    let written = std::fs::read_to_string(&rig.config).expect("written config");
    assert!(written.contains("lead = fablex\ncolead = astrax\norchestrator = gpt56solx\n"));
    assert!(written.contains("palette = darcula\n"));
    assert!(ae::config::read_identity(Some(&rig.config), None).is_ok());
    assert!(
        !rig.scratch.join("home").join(".ae").join("config").exists(),
        "checkout init must honor CONFIG_FILE"
    );
}

#[test]
fn init_never_probes_tmux_or_runs_any_discovered_program() {
    let rig = Rig::new("no-child", &["claude"]);
    let marker = rig.scratch.join("child-ran");
    let tmux = rig.bin.join("tmux");
    let body = format!(
        "#!/usr/bin/perl\nopen(my $f, '>', '{}') or die; print $f \"ran\\n\"; close($f);\n",
        marker.display()
    );
    assert!(std::fs::write(&tmux, body).is_ok());
    assert!(std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o755)).is_ok());
    let (code, stdout, stderr) = rig.run(&["init", "--yes"]);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");
    assert!(!marker.exists(), "init spawned tmux");
}

#[test]
fn an_initialized_config_launches_a_real_session_with_the_selected_profiles() {
    let rig = Rig::new("launch", &["claude", "codex"]);
    let marker = rig.install_launch_fakes();
    let (code, stdout, stderr) = rig.run_with_server(&["init", "--yes"], true);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");

    let (code, stdout, stderr) = rig.run_with_server(&["init-rig", "--no-attach"], true);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");
    assert!(stdout.contains("Session 'init-rig' started"), "{stdout}");
    let mut launched = String::new();
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(&marker) {
            launched = text;
            if launched.contains("claude\n") && launched.contains("codex\n") {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(launched.contains("claude\n"), "{launched}");
    assert!(launched.contains("codex\n"), "{launched}");
}

#[test]
fn non_tty_requires_yes_and_prints_the_proposal_without_writing() {
    let rig = Rig::new("non-tty", &["gemini"]);
    let (code, stdout, stderr) = rig.run(&["init"]);
    assert_eq!(code, Some(1));
    assert!(stdout.contains("Proposed config for"), "{stdout}");
    assert!(stdout.contains("lead = gemini\n"), "{stdout}");
    assert!(stdout.contains("layout = lead-solo\n"), "{stdout}");
    assert!(stderr.contains("rerun with --yes"), "{stderr}");
    assert!(!rig.config.exists());
}

#[test]
fn invalid_flags_and_an_empty_path_leave_the_config_absent() {
    let invalid = Rig::new("invalid", &["claude"]);
    let (code, _, stderr) = invalid.run(&["init", "--yes", "--lead", "missing"]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains(ae::init::USAGE), "{stderr}");
    assert!(!invalid.config.exists());

    let empty = Rig::new("empty", &[]);
    let (code, stdout, stderr) = empty.run(&["init", "--yes"]);
    assert_eq!(code, Some(1));
    assert!(stdout.contains("claude    not on PATH"), "{stdout}");
    assert!(stderr.contains("no supported harness on PATH"), "{stderr}");
    assert!(!empty.config.exists());
}

#[test]
fn an_existing_config_gets_an_exclusive_proposal_and_a_diff() {
    let rig = Rig::new("proposal", &["claude"]);
    assert!(std::fs::create_dir_all(rig.config.parent().unwrap_or(Path::new("."))).is_ok());
    assert!(std::fs::write(&rig.config, b"original\n").is_ok());
    let before = std::fs::read(&rig.config).expect("old config");
    let (code, stdout, stderr) = rig.run(&["init", "--yes"]);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");
    assert!(
        stdout.contains(&format!("--- {}", rig.config.display())),
        "{stdout}"
    );
    assert!(stdout.contains("@@ -1,1 +1,"), "{stdout}");
    assert_eq!(std::fs::read(&rig.config).expect("still old"), before);
    assert!(rig.config.with_file_name("config.proposed").is_file());

    let proposed = rig.config.with_file_name("config.proposed");
    let standing = std::fs::read(&proposed).expect("first proposal");
    let (code, _, stderr) = rig.run(&["init", "--yes"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("config.proposed"), "{stderr}");
    assert_eq!(std::fs::read(&rig.config).expect("still old"), before);
    assert_eq!(std::fs::read(&proposed).expect("same proposal"), standing);
}

#[test]
fn force_backs_up_then_atomically_replaces_a_regular_config() {
    let rig = Rig::new("force", &["grok", "agy"]);
    assert!(std::fs::create_dir_all(rig.config.parent().unwrap_or(Path::new("."))).is_ok());
    assert!(std::fs::write(&rig.config, b"original\n").is_ok());
    let (code, stdout, stderr) = rig.run(&["init", "--yes", "--force"]);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");
    assert!(stdout.contains("Backed up config to"), "{stdout}");
    let written = std::fs::read_to_string(&rig.config).expect("new config");
    assert!(written.contains("lead = grok46\ncolead = agy\norchestrator = grok46\n"));
    let backups: Vec<PathBuf> = std::fs::read_dir(rig.config.parent().unwrap_or(Path::new(".")))
        .expect("config dir")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().ends_with(".bak"))
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert_eq!(std::fs::read(&backups[0]).expect("backup"), b"original\n");
}

#[cfg(unix)]
#[test]
fn a_symlink_config_is_refused_without_touching_its_target() {
    let rig = Rig::new("symlink", &["claude"]);
    let target = rig.scratch.join("target");
    assert!(std::fs::write(&target, b"target\n").is_ok());
    assert!(std::fs::create_dir_all(rig.config.parent().unwrap_or(Path::new("."))).is_ok());
    assert!(std::os::unix::fs::symlink(&target, &rig.config).is_ok());
    let (code, _, stderr) = rig.run(&["init", "--yes", "--force"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("not a regular config file"), "{stderr}");
    assert_eq!(std::fs::read(&target).expect("target"), b"target\n");
    assert!(std::fs::symlink_metadata(&rig.config).is_ok_and(|meta| meta.file_type().is_symlink()));
}
