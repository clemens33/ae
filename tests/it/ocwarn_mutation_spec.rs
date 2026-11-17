//! Supplemental #310 acceptance from the approved grammar, cap and diagnostics.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures use only process-owned scratch paths and an inert executable"
)]

use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use ae::opencode_variant::{Check, check_with};
use ae::store::SourceRead;

const CAP: usize = 256 * 1024;
const SHADOW: &[u8] = br#"{"variant":{"p/m":"low"}}"#;

struct Fixture(super::cli::OwnedScratch);

impl Fixture {
    fn new(tag: &str) -> Self {
        let fixture = Self(super::cli::OwnedScratch::root("ocwarn-mutation", tag));
        for dir in [
            "bin",
            "home",
            "project",
            "xdg/opencode",
            "state/sessions/fixture",
        ] {
            fs::create_dir_all(fixture.0.join(dir)).expect("fixture directory");
        }
        std::os::unix::fs::symlink("/bin/true", fixture.0.join("bin/opencode"))
            .expect("inert executable");
        fs::write(
            fixture.0.join("config"),
            format!(
                "[profiles]\ngo = \"{} --agent build --model p/m#max\"\n[roster]\nlead = go\n[workspace]\nmain = lead\nworkers = \n",
                fixture.0.join("bin/opencode").display()
            ),
        )
        .expect("profile");
        fs::write(
            fixture.session().join("meta"),
            format!(
                "schema=2\nmode=local\nsession=fixture\norigin={}\nwork_dir={}\nconfig={}\nseat.main=lead\nprofile.main=go\nlaunch_id.main=ocwarn-mutation\n",
                fixture.0.join("project").display(),
                fixture.0.join("project").display(),
                fixture.0.join("config").display()
            ),
        )
        .expect("seat");
        fixture
    }

    fn session(&self) -> PathBuf {
        self.0.join("state/sessions/fixture")
    }

    fn seed(&self, bytes: &[u8]) {
        fs::write(self.0.join("xdg/opencode/model.json"), bytes).expect("preference");
    }

    fn run(&self, doctor: bool) -> String {
        let mut command = super::cli::ae();
        command
            .env("HOME", self.0.join("home"))
            .env("AE_HOME", self.0.join("state"))
            .env("XDG_STATE_HOME", self.0.join("xdg"))
            .env_remove("CONFIG_FILE")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .current_dir(self.0.join("project"));
        if doctor {
            command
                .arg("doctor")
                .arg("--global")
                .arg(self.0.join("config"));
        } else {
            command.arg("_run").arg(self.session()).arg("main");
        }
        let child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("real ae child");
        let output = super::cli::bounded(child, Duration::from_secs(10)).expect("bounded child");
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(if doctor { output.stdout } else { output.stderr })
            .expect("UTF-8 diagnostic")
    }
}

fn row(output: &str) -> &str {
    let rows: Vec<_> = output
        .lines()
        .filter(|line| line.split_whitespace().nth(1) == Some("variant:go"))
        .collect();
    assert_eq!(rows.len(), 1, "one variant row: {output}");
    rows[0]
}

fn padded_shadow(length: usize) -> Vec<u8> {
    let mut bytes = SHADOW.to_vec();
    bytes.resize(length, b' ');
    bytes
}

#[test]
fn real_regular_preferences_within_the_promised_cap_are_checked() {
    // Approved plan item 3 promises 256 KiB, including the boundary. Exercise
    // the real reader and its doctor/launch consumers, not a constant pin.
    for length in [4096, CAP] {
        let fixture = Fixture::new(&format!("size-{length}"));
        fixture.seed(&padded_shadow(length));
        let doctor = fixture.run(true);
        let variant = row(&doctor);
        assert!(variant.starts_with("WARN "), "{doctor}");
        for value in ["p/m", "max", "low"] {
            assert!(variant.contains(value), "{variant}");
        }
        // The approved launch channel is stderr, as in the frozen main spec.
        let launch = fixture.run(false);
        let lines: Vec<_> = launch
            .lines()
            .filter(|line| line.contains("saved"))
            .collect();
        assert_eq!(lines.len(), 1, "{launch}");
        for value in ["p/m", "max", "low", "will use"] {
            assert!(lines[0].contains(value), "{launch}");
        }
    }
}

#[test]
fn malformed_model_words_are_unknown_before_reading_native_preferences() {
    // OpenCode v2.3.0 ConfigModel.Selection Short grammar:
    // ^[^/#]+/[^#]+(?:#[^#]+)?$ (model.ts:9-11,20).
    // https://github.com/anomalyco/opencode/blob/v2.3.0/packages/schema/src/config/model.ts#L9-L20
    // The accepted plan makes a # word outside this grammar unknown, not clear
    // or shadowed. Only external I/O is injected here.
    for model in ["/m#v", "a#b/m#v", "p/#v", "p/m#", "p/m#a#b"] {
        let reads = Cell::new(0);
        let check = check_with(
            &format!("opencode --agent build --model '{model}'"),
            &|_| Some("/fixture".into()),
            &|_| {
                reads.set(reads.get() + 1);
                Ok(SourceRead::Ready(SHADOW.to_vec()))
            },
        );
        assert!(matches!(check, Check::Unknown(_)), "{model}: {check:?}");
        assert_eq!(reads.get(), 0, "no native-file guess for {model}");
    }
    // Sensitivity control: this same external reader is reached for a valid
    // nested model ID, which the upstream grammar permits.
    let reads = Cell::new(0);
    let check = check_with(
        "opencode --agent build --model p/nested/m#max",
        &|_| Some("/fixture".into()),
        &|_| {
            reads.set(reads.get() + 1);
            Ok(SourceRead::Ready(
                br#"{"variant":{"p/nested/m":"low"}}"#.to_vec(),
            ))
        },
    );
    assert!(matches!(check, Check::Shadowed { .. }), "{check:?}");
    assert_eq!(reads.get(), 1);
}

#[test]
fn unknown_doctor_details_distinguish_bad_shape_from_the_size_limit() {
    // Approved plan item 6 promises a reason after unknown:, from distinct
    // closed causes. Preserve the distinction without freezing prose wording.
    let fixture = Fixture::new("unknown-reasons");
    let mut details = Vec::new();
    for bytes in [br"[]".to_vec(), padded_shadow(CAP + 1)] {
        fixture.seed(&bytes);
        let doctor = fixture.run(true);
        let variant = row(&doctor);
        assert!(!variant.starts_with("FAIL "), "{doctor}");
        let (_, detail) = variant.split_once("unknown:").expect("unknown row");
        assert!(
            !detail.trim().is_empty(),
            "doctor must explain uncertainty: {variant}"
        );
        details.push(detail.trim().to_owned());
        assert!(
            !fixture.run(false).contains("saved"),
            "unknown starts silently"
        );
    }
    assert_ne!(details[0], details[1], "doctor must identify the cause");
}
