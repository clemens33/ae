//! #310 independent acceptance. Oracle: owner-approved plan-r2.3 / D9.
//! Current resolved profile inventory only; no native selection inference.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect only process-owned scratch homes"
)]

use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::PathBuf;

use ae::doctor::{Facts, Level, ProfileFacts, Report, ReseatCover, Row, report};

const MODEL: &str = "opencode-go/glm-5.3-flash#max";
const GOLDEN: &str = "opencode; current profile requests \"opencode-go/glm-5.3-flash#max\"; effective variant and availability cannot be verified; native saved preferences or a session selection may override it";
const TAIL: &str = "; effective variant and availability cannot be verified; native saved preferences or a session selection may override it";

fn facts(command: &str) -> Facts {
    Facts {
        version: "2026.9.1".into(),
        core: Some(PathBuf::from("/opt/ae/versions/1/ae-core")),
        core_writable: Some(false),
        core_published: true,
        tmux: Some(PathBuf::from("/usr/bin/tmux")),
        tmux_floor: ae::tmux_floor::Probe::Executable("3.7b".into()),
        boot: Some(1_789_105_855),
        git: Some(PathBuf::from("/usr/bin/git")),
        config: PathBuf::from("/fixture/config"),
        config_error: None,
        local_config: None,
        now: 1_789_200_000,
        quota: vec![],
        main: Some("lead".into()),
        workers: Some(String::new()),
        fleet_order: String::new(),
        fleet_order_error: None,
        profiles: vec![ProfileFacts {
            profile: "go".into(),
            command: command.into(),
            executable: Some("opencode".into()),
            resolves: true,
            resolution_error: None,
        }],
        sessions_dir: PathBuf::from("/fixture/sessions"),
        worktrees_dir: PathBuf::from("/fixture/worktrees"),
        sessions: vec![],
        seats: vec![],
        damaged_records: vec![],
        bindings: vec![],
        reseat_cover: ReseatCover::Off,
    }
}

fn agent(report: &Report) -> &Row {
    let rows: Vec<_> = report
        .rows()
        .iter()
        .filter(|row| row.label == "agent:go")
        .collect();
    assert_eq!(rows.len(), 1, "one existing inventory row");
    rows[0]
}

#[test]
fn current_profile_request_changes_only_the_existing_ok_detail() {
    let control = report(&facts(
        "opencode --agent build --model opencode-go/glm-5.3-flash",
    ));
    let actual = report(&facts(&format!("opencode --agent build --model {MODEL}")));
    let mut expected = control.rows().to_vec();
    let row = expected
        .iter_mut()
        .find(|row| row.label == "agent:go")
        .expect("agent row");
    assert_eq!(row.level, Level::Ok);
    assert_eq!(row.detail, "opencode");
    row.detail = GOLDEN.into();
    assert_eq!(
        actual.rows(),
        expected,
        "same rows/order/levels; only detail changes"
    );
    assert_eq!(actual.failures(), control.failures());
    assert_eq!(actual.warnings(), control.warnings());
    assert_eq!(actual.exit_code(), control.exit_code());
    let expected_render = control.render().replacen(
        "agent:go       opencode\n",
        &format!("agent:go       {GOLDEN}\n"),
        1,
    );
    assert_ne!(
        expected_render,
        control.render(),
        "the render oracle replaced the actual row"
    );
    assert_eq!(actual.render(), expected_render);
}

#[test]
fn nonqualifying_and_refused_commands_keep_the_entire_report() {
    let control = report(&facts("opencode"));
    // Complete validation includes off-diagonal agent/standalone failures,
    // not just model extraction or the refusal operand list.
    for command in [
        "opencode",
        "opencode --agent build",
        "opencode --agent build --model p/m",
        "codex --model p/m#max",
        "claude --model p/m#max",
        "echo --model p/m#max",
        "opencode --model p/m#max",
        "opencode -m p/m#max",
        "opencode --agent build --model p/m#max --variant max",
        "opencode --agent build --model p/m#max --variant=max",
        "opencode --agent build --model p/m#max --server x",
        "opencode --agent build --model p/m#max --server=x",
        "opencode --agent build --model p/m#max --standalone --standalone",
        "opencode --agent build --model p/m#max --standalone=true",
        "opencode --agent build --model p/m#max mini",
        "opencode --agent build -- --model p/m#max",
        "opencode --agent build --model p/m#max --model p/other",
        "opencode --agent build -m p/m#max --model p/other",
        "opencode --agent build --model p/m#max --agent plan",
        "opencode --agent '' --model p/m#max",
        "opencode --agent '$AGENT' --model p/m#max",
        "opencode --agent 'bad\nagent' --model p/m#max",
        "opencode --agent build -mp/m#max",
        "opencode --agent build -m=p/m#max",
        "opencode --agent build --model ''",
        "opencode --agent build --model=",
        "opencode --agent build --model '$MODEL#max'",
        "opencode --agent build --model 'p/m#max\nnext'",
        "opencode --agent build --model 'p/m#max\tnext'",
        "opencode --agent build --model 'p/m#max\rnext'",
        "opencode --agent build --model p/m#max ; true",
        "opencode --agent build --model 'p/m#max",
        "OPENCODE_CONFIG=x opencode --agent build --model p/m#max",
        "OPENCODE_CONFIG_CONTENT='{}' opencode --agent build --model p/m#max",
        "env -u OPENCODE_CONFIG opencode --agent build --model p/m#max",
        "env -u OPENCODE_CONFIG_CONTENT opencode --agent build --model p/m#max",
    ] {
        assert_eq!(report(&facts(command)), control, "{command:?}");
    }
}

#[test]
fn all_failure_arms_keep_their_original_detail_and_status() {
    let mut input = facts("opencode --agent build --model p/m#max");
    for (executable, resolves, error, detail) in [
        (
            Some("opencode"),
            true,
            Some("client resolution failed"),
            "client resolution failed",
        ),
        (
            None,
            true,
            None,
            "could not determine executable from 'opencode --agent build --model p/m#max'",
        ),
        (
            Some("opencode"),
            false,
            None,
            "command 'opencode' not found",
        ),
    ] {
        input.profiles[0].executable = executable.map(str::to_owned);
        input.profiles[0].resolves = resolves;
        input.profiles[0].resolution_error = error.map(str::to_owned);
        let actual = report(&input);
        assert_eq!(
            agent(&actual),
            &Row {
                level: Level::Fail,
                label: "agent:go".into(),
                detail: detail.into()
            }
        );
        assert_eq!(actual.failures(), 1);
        assert_eq!(actual.exit_code(), 1);
        assert!(!actual.render().contains("current profile requests"));
    }
}

#[test]
fn rust_debug_quotes_format_characters_before_rendering() {
    assert_eq!(format!("{:?}", "p/m#\u{202e}"), "\"p/m#\\u{202e}\"");
    assert_eq!(format!("{:?}", "a b#c\"d\\tail"), "\"a b#c\\\"d\\\\tail\"");
}

#[test]
fn lexed_opaque_values_are_quoted_safely_without_suffix_grammar() {
    for (word, escaped) in [
        ("p/m#", "\"p/m#\""),
        ("p/m#a#b", "\"p/m#a#b\""),
        ("'a b#c\"d'", "\"a b#c\\\"d\""),
        ("'p/m#\\tail'", "\"p/m#\\\\tail\""),
        ("p/m\\#max", "\"p/m#max\""),
        ("'p/m#\u{202e}'", "\"p/m#\\u{202e}\""),
        ("'pré/m#é'", "\"pré/m#é\""),
    ] {
        let actual = report(&facts(&format!("opencode --agent build --model {word}")));
        let detail = &agent(&actual).detail;
        assert_eq!(
            detail,
            &format!("opencode; current profile requests {escaped}{TAIL}"),
            "{word:?}"
        );
        assert!(!detail.contains(['\n', '\r', '\t', '\u{202e}']));
        assert_eq!(
            actual
                .render()
                .lines()
                .filter(|line| line.contains("agent:go"))
                .count(),
            1
        );
    }
}

struct Rig(super::cli::OwnedScratch);

impl Rig {
    fn new(tag: &str) -> Self {
        let rig = Self(super::cli::OwnedScratch::root("gv", tag));
        for dir in ["home", "state/sessions", "plain", "local/.ae", "bin"] {
            fs::create_dir_all(rig.0.join(dir)).expect("fixture directory");
        }
        // This fake never delegates. If anything executes it, its marker and
        // nonzero status make the fixture's absence assertion meaningful.
        let fake = rig.0.join("bin/opencode");
        fs::write(&fake, format!("#!/usr/bin/perl\nopen(my $f, '>', '{}') or die $!; print $f 'executed'; exit 77;\n", rig.0.join("executed").display())).expect("fake executable");
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("executable mode");
        fs::write(rig.0.join("config"), format!("[profiles]\ngo = \"{} --agent build --model p/m#low\"\n[roster]\nlead = go\n[workspace]\nmain = lead\nworkers = \n", fake.display())).expect("global config");
        fs::write(
            rig.0.join("local/.ae/config"),
            format!(
                "[profiles]\ngo = \"{} --agent build --model p/m#max\"\n",
                fake.display()
            ),
        )
        .expect("layered config");
        rig
    }

    fn doctor(&self, directory: &str) -> (i32, String) {
        let child = super::cli::ae()
            .env("HOME", self.0.join("home"))
            .env("AE_HOME", self.0.join("state"))
            .env("XDG_CONFIG_HOME", self.0.join("home/.config"))
            .env("XDG_STATE_HOME", self.0.join("home/.local/state"))
            .env("XDG_DATA_HOME", self.0.join("home/.local/share"))
            .env_remove("CONFIG_FILE")
            .current_dir(self.0.join(directory))
            .arg("doctor")
            .arg("--global")
            .arg(self.0.join("config"))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("real sandbox doctor");
        let output = super::cli::bounded(child, std::time::Duration::from_secs(10))
            .expect("doctor exits within ten seconds and owned child is reaped");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            !self.0.join("executed").exists(),
            "fake OpenCode must not execute"
        );
        (
            output.status.code().expect("ordinary exit"),
            String::from_utf8(output.stdout).expect("UTF-8 doctor"),
        )
    }
}

#[test]
fn real_doctor_uses_the_layered_current_profile() {
    let rig = Rig::new("layer");
    let (_, global) = rig.doctor("plain");
    let (_, local) = rig.doctor("local");
    for (text, model) in [(global, "p/m#low"), (local, "p/m#max")] {
        let rows: Vec<_> = text
            .lines()
            .filter(|line| line.split_whitespace().nth(1) == Some("agent:go"))
            .collect();
        assert_eq!(rows.len(), 1, "{text}");
        assert_eq!(
            rows[0],
            format!(
                "OK    agent:go       {}; current profile requests {model:?}{TAIL}",
                rig.0.join("bin/opencode").display()
            )
        );
        assert!(text.contains("Summary: 0 failure(s),"), "{text}");
    }
}

#[test]
fn seeded_native_files_do_not_change_output_or_post_run_state() {
    let rig = Rig::new("seeds");
    let baseline = rig.doctor("plain");
    let files: Vec<_> = [
        "home/.local/state/opencode/model.json",
        "home/.local/state/opencode/kv.json",
        "home/.local/share/opencode/storage/session/session.json",
    ]
    .into_iter()
    .map(|path| rig.0.join(path))
    .collect();
    assert!(files.iter().all(|path| !path.exists()));
    for selection in ["default", "high", "max"] {
        for file in &files {
            fs::create_dir_all(file.parent().expect("seed parent")).expect("seed directory");
            fs::write(file, format!("{{\"selection\":\"{selection}\"}}\n")).expect("seed bytes");
        }
        let before: Vec<_> = files
            .iter()
            .map(|file| {
                let metadata = fs::metadata(file).expect("seed metadata");
                (
                    fs::read(file).expect("seed read"),
                    metadata.modified().expect("mtime"),
                    metadata.ino(),
                    metadata.dev(),
                )
            })
            .collect();
        assert_eq!(
            rig.doctor("plain"),
            baseline,
            "output independent of {selection} seed"
        );
        for (file, expected) in files.iter().zip(before) {
            let metadata = fs::metadata(file).expect("post-run metadata");
            assert_eq!(
                (
                    fs::read(file).expect("post-run read"),
                    metadata.modified().expect("mtime"),
                    metadata.ino(),
                    metadata.dev()
                ),
                expected
            );
        }
    }
}
