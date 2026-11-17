//! Independent #310 acceptance: the brief and `OpenCode` v2.3.0 preference rules.
//! Real doctor and _run processes use private homes and an inert executable.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures read and write only process-owned scratch paths"
)]

use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

struct Rig(super::cli::OwnedScratch);

impl Rig {
    fn new(tag: &str) -> Self {
        let rig = Self(super::cli::OwnedScratch::root("ocwarn", tag));
        for dir in [
            "home",
            "state/sessions/fixture",
            "project",
            "bin",
            "xdg/opencode",
        ] {
            fs::create_dir_all(rig.0.join(dir)).expect("fixture directory");
        }
        // No tool script, account or network request: true ignores every argv.
        std::os::unix::fs::symlink("/bin/true", rig.0.join("bin/opencode"))
            .expect("inert OpenCode substitute");
        rig.profile("", "p/m#max");
        let session = rig.session();
        fs::write(
            session.join("meta"),
            format!(
                "schema=2\nmode=local\nsession=fixture\norigin={}\nwork_dir={}\nconfig={}\nseat.main=lead\nprofile.main=go\nlaunch_id.main=ocwarn-token\n",
                rig.0.join("project").display(),
                rig.0.join("project").display(),
                rig.0.join("config").display()
            ),
        )
        .expect("fixture seat");
        rig
    }

    fn session(&self) -> PathBuf {
        self.0.join("state/sessions/fixture")
    }

    fn profile(&self, prefix: &str, model: &str) {
        fs::write(
            self.0.join("config"),
            format!(
                "[profiles]\ngo = \"{prefix}{} --agent build --model {model}\"\n[roster]\nlead = go\n[workspace]\nmain = lead\nworkers = \n",
                self.0.join("bin/opencode").display()
            ),
        )
        .expect("fixture profile");
    }

    fn model_file(&self) -> PathBuf {
        self.0.join("xdg/opencode/model.json")
    }

    fn seed(&self, bytes: &[u8]) {
        fs::write(self.model_file(), bytes).expect("native preference fixture");
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = super::cli::ae();
        cmd.env("HOME", self.0.join("home"))
            .env("AE_HOME", self.0.join("state"))
            .env("XDG_STATE_HOME", self.0.join("xdg"))
            .env("OCWARN_STATE_ROOT", self.0.join("alternate"))
            .env_remove("CONFIG_FILE")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .current_dir(self.0.join("project"))
            .args(args);
        if args.first() == Some(&"doctor") {
            cmd.arg("--global").arg(self.0.join("config"));
        } else {
            cmd.arg(self.session()).arg("main");
        }
        let child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("real ae child");
        let out = super::cli::bounded(child, Duration::from_secs(10))
            .expect("ae must finish and the owned child must be reaped");
        assert!(out.status.success(), "{out:?}");
        out
    }

    fn doctor(&self) -> String {
        String::from_utf8(self.run(&["doctor"]).stdout).expect("UTF-8 doctor output")
    }

    fn launch(&self) -> String {
        // The approved plan puts the launch warning on stderr. A different
        // channel requires an explicit acceptance-spec change.
        String::from_utf8(self.run(&["_run"]).stderr).expect("UTF-8 launch output")
    }

    fn plan(&self) -> ae::json::Value {
        let out = self.run(&["_run", "--print"]);
        ae::json::parse(&String::from_utf8(out.stdout).expect("UTF-8 plan"))
            .expect("JSON launch plan")
    }
}

fn variant_row(text: &str) -> &str {
    let rows: Vec<_> = text
        .lines()
        .filter(|row| row.split_whitespace().nth(1) == Some("variant:go"))
        .collect();
    assert_eq!(rows.len(), 1, "one explicit-variant profile row: {text}");
    rows[0]
}

fn shadow_line(text: &str, saved: &str) {
    let rows: Vec<_> = text
        .lines()
        .filter(|line| line.contains("p/m") && line.contains("saved"))
        .collect();
    assert_eq!(rows.len(), 1, "one visible launch warning: {text}");
    let row = rows[0];
    for word in ["p/m", "max", saved, "OpenCode", "/variants"] {
        assert!(row.contains(word), "missing {word:?} in {row:?}");
    }
    assert!(
        row.contains("will use") && row.contains("saved"),
        "the warning must explain which preference wins: {row}"
    );
}

fn unchanged(path: &Path, before: &(Vec<u8>, u64, u64, std::time::SystemTime)) {
    let meta = fs::metadata(path).expect("preference still exists");
    assert_eq!(
        &(
            fs::read(path).expect("preference bytes"),
            meta.dev(),
            meta.ino(),
            meta.modified().expect("mtime")
        ),
        before,
        "ae must not write, move or replace model.json"
    );
    let siblings: Vec<_> = fs::read_dir(path.parent().expect("parent"))
        .expect("preference directory")
        .map(|entry| entry.expect("directory entry").file_name())
        .collect();
    assert_eq!(
        siblings,
        vec![std::ffi::OsString::from("model.json")],
        "no lock or side file"
    );
}

#[test]
fn missing_file_and_missing_key_are_clear_without_a_launch_warning() {
    let rig = Rig::new("missing");
    let absent = rig.doctor();
    assert!(variant_row(&absent).starts_with("OK "), "{absent}");
    assert!(!variant_row(&absent).contains("unknown"), "{absent}");
    assert!(!rig.launch().contains("saved"));
    assert!(!rig.model_file().exists(), "no preference file is created");

    let other = Rig::new("no-key");
    other.seed(br#"{"recent":[],"variant":{"other/model":"low"}}"#);
    let no_key = other.doctor();
    assert_eq!(variant_row(&absent), variant_row(&no_key));
    assert!(!other.launch().contains("saved"));
}

#[test]
fn same_saved_variant_is_clear_in_doctor_and_silent_at_start() {
    let rig = Rig::new("same");
    rig.seed(br#"{"variant":{"p/m":"max"}}"#);
    let out = rig.doctor();
    assert!(variant_row(&out).starts_with("OK "), "{out}");
    assert!(!variant_row(&out).contains("unknown"), "{out}");
    assert!(!rig.launch().contains("saved"));
}

#[test]
fn different_and_default_preferences_warn_without_mutation_or_launch_refusal() {
    for saved in ["low", "default"] {
        let rig = Rig::new(saved);
        rig.seed(format!("{{\"variant\":{{\"p/m\":\"{saved}\"}}}}").as_bytes());
        let file = rig.model_file();
        let meta = fs::metadata(&file).expect("native file");
        let before = (
            fs::read(&file).expect("bytes"),
            meta.dev(),
            meta.ino(),
            meta.modified().expect("mtime"),
        );
        let out = rig.doctor();
        let row = variant_row(&out);
        assert!(row.starts_with("WARN "), "{out}");
        for word in ["p/m", "max", saved, "/variants"] {
            assert!(row.contains(word), "missing {word:?}: {row}");
        }
        unchanged(&file, &before);
        shadow_line(&rig.launch(), saved);
        unchanged(&file, &before);
    }
}

#[test]
fn damaged_and_oversized_preferences_are_unknown_and_never_block_start() {
    for (tag, bytes) in [
        ("bad-json", b"{broken".to_vec()),
        ("wrong-shape", br#"{"variant":[]}"#.to_vec()),
        ("oversized", vec![b' '; 2 * 1024 * 1024]),
    ] {
        let rig = Rig::new(tag);
        rig.seed(&bytes);
        let out = rig.doctor();
        assert!(
            variant_row(&out).to_lowercase().contains("unknown"),
            "{out}"
        );
        assert!(!rig.launch().contains("saved"));
        assert_eq!(
            fs::read(rig.model_file()).expect("kept native bytes"),
            bytes
        );
    }
}

#[test]
fn absent_or_matching_preferences_do_not_change_the_launch_argv_or_environment() {
    let rig = Rig::new("plan");
    let before = rig.plan();
    rig.seed(br#"{"variant":{"p/m":"low"}}"#);
    let after = rig.plan();
    for field in ["argv", "env_clear", "env_unset", "env_set"] {
        assert_eq!(before.get(field), after.get(field), "unchanged {field}");
        assert!(before.get(field).is_some(), "the plan exposes {field}");
    }
    let sets = after.get("env_set").expect("environment plan").render();
    assert!(
        sets.contains("p/m#max"),
        "the original variant is passed unchanged: {sets}"
    );
    assert!(
        !sets.contains("XDG_STATE_HOME"),
        "ae must not scope native state: {sets}"
    );
}

#[test]
fn doctor_and_launch_honor_expanded_profile_state_overrides() {
    let rig = Rig::new("profile-env");
    rig.seed(br#"{"variant":{"p/m":"low"}}"#);
    let alternate = rig.0.join("alternate/opencode");
    fs::create_dir_all(&alternate).expect("alternate state");
    fs::write(
        alternate.join("model.json"),
        br#"{"variant":{"p/m":"max"}}"#,
    )
    .expect("matching alternate");
    rig.profile("env XDG_STATE_HOME=$OCWARN_STATE_ROOT ", "p/m#max");
    assert!(variant_row(&rig.doctor()).starts_with("OK "));
    assert!(!rig.launch().contains("saved"));

    let opposed = Rig::new("profile-env-opposed");
    opposed.seed(br#"{"variant":{"p/m":"max"}}"#);
    let alternate = opposed.0.join("alternate/opencode");
    fs::create_dir_all(&alternate).expect("alternate state");
    fs::write(
        alternate.join("model.json"),
        br#"{"variant":{"p/m":"low"}}"#,
    )
    .expect("opposed alternate");
    opposed.profile("env XDG_STATE_HOME=$OCWARN_STATE_ROOT ", "p/m#max");
    assert!(variant_row(&opposed.doctor()).starts_with("WARN "));
    shadow_line(&opposed.launch(), "low");
}

#[test]
fn exact_session_resume_does_not_claim_saved_preference_wins() {
    let rig = Rig::new("resume");
    rig.seed(br#"{"variant":{"p/m":"low"}}"#);
    fs::write(rig.session().join("launch.main.started"), "").expect("start marker");
    let meta = rig.session().join("meta");
    let mut body = fs::read_to_string(&meta).expect("meta");
    body.push_str("harness_session.main=existing-session\n");
    fs::write(meta, body).expect("existing session");
    let text = rig.launch();
    assert!(
        text.contains(ae::run::RESUMING),
        "fixture must reach resume: {text}"
    );
    assert!(
        !text.contains("saved"),
        "existing selection outranks model.json: {text}"
    );
}

#[test]
fn profiles_without_a_variant_do_not_get_a_variant_row() {
    let rig = Rig::new("no-variant");
    rig.profile("", "p/m");
    rig.seed(br#"{"variant":{"p/m":"low"}}"#);
    assert!(!rig.doctor().contains("variant:go"));
    assert!(!rig.launch().contains("saved"));
}

#[test]
fn hostile_saved_text_is_escaped_on_one_terminal_line() {
    let rig = Rig::new("hostile");
    rig.seed(br#"{"variant":{"p/m":"low\n\u001b[31m\u202e"}}"#);
    let text = rig.launch();
    let lines: Vec<_> = text
        .lines()
        .filter(|line| line.contains("p/m") && line.contains("saved"))
        .collect();
    assert_eq!(lines.len(), 1, "one intact diagnostic: {text:?}");
    assert!(
        !lines[0].chars().any(char::is_control),
        "inert terminal text: {text:?}"
    );
    assert!(!text.contains('\u{202e}'), "bidi control escaped: {text:?}");
    assert!(
        lines[0].contains("\\n") && lines[0].contains("\\u{1b}"),
        "saved bytes stay visible as escapes: {text:?}"
    );
}
