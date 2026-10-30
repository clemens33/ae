//! #276 B acceptance, authored from B1–B4 and local `OpenCode` 2.2.1 measurements.
//! Fresh max / resumed low is a measured counterexample: configuration is not
//! a model-change resume mechanism. Captured busy frames remain Unknown.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures own and inspect private scratch artifacts"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use ae::harness_state::{HarnessIdentity, HarnessState, classify, observed_identity};
use ae::json::{Value, parse};
use ae::launch::inject_ae_context;
use ae::launch_cmd::{effort_flag, sole_flag_value};
use ae::tool::{Composed, ComposerAnchor, DialogSig, ToolKind};

const MODEL: &str = "opencode-go/probe#max";
const ID: &str = "ses_e7961d6d8ffe4C1tKWN8vrHCv1";
// Frozen measured structure; adapter wiring remains pinned by tool's table test.
const HEAVY: Composed = Composed {
    anchor: ComposerAnchor::HeavyRail,
    markers: &["Ask anything…"],
    dialog: DialogSig {
        dismiss: "esc",
        body: "Search",
        gap: 3,
    },
};

fn config(dir: &Path, slot: &str) -> Value {
    parse(&fs::read_to_string(dir.join(format!("opencode.{slot}.json"))).expect("seat JSON"))
        .expect("valid JSON")
}

fn selection(cmd: &str) -> Value {
    let args = runtime(cmd);
    let values: Vec<_> = args
        .iter()
        .filter_map(|arg| arg.strip_prefix("OPENCODE_CONFIG_CONTENT="))
        .collect();
    assert!(values.len() <= 1, "one inline layer at most: {args:?}");
    parse(values.first().copied().unwrap_or("{}")).expect("inline selection JSON")
}

fn assert_pointer(cmd: &str, dir: &Path, slot: &str) {
    let args = runtime(cmd);
    let pointers: Vec<_> = args
        .iter()
        .filter_map(|arg| arg.strip_prefix("OPENCODE_CONFIG="))
        .collect();
    assert_eq!(
        pointers,
        [dir.join(format!("opencode.{slot}.json"))
            .display()
            .to_string()]
    );
}

fn assert_selection(cfg: &Value, agent: Option<&str>, model: Option<&str>) {
    assert_eq!(cfg.get_str("default_agent"), agent, "{cfg:?}");
    assert_eq!(cfg.get_str("model"), model, "{cfg:?}");
    if let Some(agent) = agent {
        assert_eq!(
            cfg.get("agents")
                .and_then(|v| v.get(agent))
                .and_then(|v| v.get_str("model")),
            model,
            "selected agent must carry the requested model: {cfg:?}"
        );
    }
    assert!(
        cfg.get("instructions").is_none(),
        "context stays in pointer file"
    );
}

fn assert_config(dir: &Path, slot: &str, ctx: &str) {
    let cfg = config(dir, slot);
    assert!(
        cfg.get("default_agent").is_none(),
        "pointer is instructions-only"
    );
    assert!(cfg.get("model").is_none(), "pointer is instructions-only");
    assert!(cfg.get("agents").is_none(), "pointer is instructions-only");
    let md = dir.join(format!("opencode.{slot}.md"));
    assert_eq!(
        cfg.get("instructions"),
        Some(&Value::Arr(vec![Value::Str(md.display().to_string())]))
    );
    assert_eq!(
        fs::read_to_string(&md).expect("context"),
        format!("{ctx}\n")
    );
    for path in [md, dir.join(format!("opencode.{slot}.json"))] {
        assert_eq!(
            fs::metadata(path)
                .expect("published file")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

fn runtime(cmd: &str) -> Vec<String> {
    ae::words::split(cmd, &|_| None).expect("runtime shell words")
}

#[test]
fn original_flags_become_seat_config_and_standalone_runtime() {
    let scratch = super::cli::OwnedScratch::root("oc2b", "translation");
    for (index, original) in [
        "opencode --agent build --model 'opencode-go/probe#max'",
        "opencode --agent=build --model=opencode-go/probe#max",
        "opencode -m 'opencode-go/probe#max' --agent build",
    ]
    .into_iter()
    .enumerate()
    {
        let slot = format!("spawned.{index}");
        let injected = inject_ae_context(original, scratch.path(), &slot, "CANARY-A", "");
        assert_eq!(injected.warning, None);
        let args = runtime(&injected.cmd);
        assert!(
            args.ends_with(&["opencode".to_owned(), "--standalone".to_owned()]),
            "{args:?}"
        );
        assert!(
            args.contains(&format!(
                "OPENCODE_CONFIG={}",
                scratch.join(format!("opencode.{slot}.json")).display()
            )),
            "{args:?}"
        );
        assert_eq!(
            args.iter().filter(|v| v.as_str() == "--standalone").count(),
            1
        );
        assert_selection(&selection(&injected.cmd), Some("build"), Some(MODEL));
        assert_config(scratch.path(), &slot, "CANARY-A");
        assert_eq!(
            sole_flag_value(original, &["--model", "-m"]).as_deref(),
            Some(MODEL)
        );
    }
}

#[test]
fn prefixes_executable_and_exact_session_survive_translation() {
    let scratch = super::cli::OwnedScratch::root("oc2b", "prefix");
    let original = format!(
        "X='a b' env -u UNUSED /tmp/oc2b-bin/opencode --agent build -m '{MODEL}' --standalone --session {ID}"
    );
    let injected = inject_ae_context(&original, scratch.path(), "main", "PREFIX-CANARY", "");
    let args = runtime(&injected.cmd);
    assert_eq!(injected.warning, None);
    assert!(args.iter().any(|s| s == "X=a b"), "{args:?}");
    assert!(
        args.iter().any(|s| s == "/tmp/oc2b-bin/opencode"),
        "{args:?}"
    );
    assert!(args.windows(2).any(|w| w == ["-u", "UNUSED"]), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--session", ID]), "{args:?}");
    assert!(
        args.windows(3)
            .any(|w| w == ["--standalone", "--session", ID]),
        "existing standalone stays in place: {args:?}"
    );
    assert_eq!(args.iter().filter(|s| *s == "--standalone").count(), 1);
    for flag in ["--agent", "--model", "-m"] {
        assert!(!args.iter().any(|s| s == flag), "{args:?}");
    }
    assert_selection(&selection(&injected.cmd), Some("build"), Some(MODEL));
    assert_config(scratch.path(), "main", "PREFIX-CANARY");
}

#[test]
fn bare_command_adds_standalone_without_inventing_agent_or_model() {
    let scratch = super::cli::OwnedScratch::root("oc2b", "bare");
    let injected = inject_ae_context("opencode", scratch.path(), "main", "BARE", "");
    assert!(runtime(&injected.cmd).ends_with(&["opencode".to_owned(), "--standalone".to_owned()]));
    assert_selection(&selection(&injected.cmd), None, None);
    assert_config(scratch.path(), "main", "BARE");
    let agent_only = inject_ae_context("opencode --agent plan", scratch.path(), "main", "BARE", "");
    assert_selection(&selection(&agent_only.cmd), Some("plan"), None);
}

#[test]
fn two_seat_configs_and_canaries_remain_independent_across_regeneration() {
    let scratch = super::cli::OwnedScratch::root("oc2b", "isolation");
    let dir = scratch.join("a 'quoted' path\nwith newline");
    fs::create_dir_all(&dir).expect("context directory");
    let a = inject_ae_context(
        "opencode --agent build -m 'opencode-go/probe#max'",
        &dir,
        "spawned.0",
        "CANARY-A\n'quoted'",
        "",
    );
    let before = fs::read(dir.join("opencode.spawned.0.json")).expect("A config");
    let a_ctx = fs::read(dir.join("opencode.spawned.0.md")).expect("A context");
    let b = inject_ae_context(
        "opencode --agent plan --model 'openrouter/probe#low'",
        &dir,
        "spawned.1",
        "CANARY-B",
        "",
    );
    assert_selection(&selection(&a.cmd), Some("build"), Some(MODEL));
    assert_pointer(&a.cmd, &dir, "spawned.0");
    assert_pointer(&b.cmd, &dir, "spawned.1");
    assert_selection(
        &selection(&b.cmd),
        Some("plan"),
        Some("openrouter/probe#low"),
    );
    assert_config(&dir, "spawned.0", "CANARY-A\n'quoted'");
    assert_config(&dir, "spawned.1", "CANARY-B");
    let b2 = inject_ae_context(
        "opencode --agent plan --model 'openrouter/other#max'",
        &dir,
        "spawned.1",
        "CANARY-B2",
        "",
    );
    assert_eq!(
        fs::read(dir.join("opencode.spawned.0.json")).expect("A config"),
        before
    );
    assert_eq!(
        fs::read(dir.join("opencode.spawned.0.md")).expect("A context"),
        a_ctx
    );
    assert_selection(
        &selection(&b2.cmd),
        Some("plan"),
        Some("openrouter/other#max"),
    );
    assert_config(&dir, "spawned.1", "CANARY-B2");
}

#[test]
fn v2_frames_preserve_measured_composer_identity_and_unknown_busy_contract() {
    for (frame, model) in [
        (
            include_str!("../fixtures/opencode-v2/full-idle-80x24.txt"),
            "GLM-5.3-Flash OpenCode Go",
        ),
        (
            include_str!("../fixtures/opencode-v2/full-idle-80x24.esc"),
            "GLM-5.3-Flash OpenCode Go",
        ),
        (
            include_str!("../fixtures/opencode-v2/full-idle-120x40.txt"),
            "GLM-5.3-Flash OpenCode Go",
        ),
        (
            include_str!("../fixtures/opencode-v2/full-idle-120x40.esc"),
            "GLM-5.3-Flash OpenCode Go",
        ),
        (
            include_str!("../fixtures/opencode-v2/local-ui-15.txt"),
            "Probe Local deterministic probe",
        ),
        (
            include_str!("../fixtures/opencode-v2/local-ui-15.esc"),
            "Probe Local deterministic probe",
        ),
        (
            include_str!("../fixtures/opencode-v2/local-ui-30.txt"),
            "Probe Local deterministic probe",
        ),
        (
            include_str!("../fixtures/opencode-v2/local-ui-30.esc"),
            "Probe Local deterministic probe",
        ),
        (
            include_str!("../fixtures/opencode-v2/local-ui-54.txt"),
            "Probe Local deterministic probe",
        ),
        (
            include_str!("../fixtures/opencode-v2/local-ui-54.esc"),
            "Probe Local deterministic probe",
        ),
        (
            include_str!("../fixtures/opencode-v2/resume-full-120x40.txt"),
            "Probe Local deterministic probe",
        ),
        (
            include_str!("../fixtures/opencode-v2/resume-full-120x40.esc"),
            "Probe Local deterministic probe",
        ),
    ] {
        assert!(
            ae::deliver::region::composed_ui(frame, HEAVY),
            "measured full TUI composes"
        );
        assert_eq!(
            observed_identity(frame, ToolKind::OpenCode),
            HarnessIdentity {
                model: Some(model.to_owned()),
                effort: Some("max".to_owned()),
            }
        );
        assert_eq!(classify(frame, ToolKind::OpenCode), HarnessState::Unknown);
    }
}

#[test]
fn boot_and_transcript_only_frames_prove_no_current_identity() {
    for frame in [
        "",
        "\n".repeat(40).as_str(),
        include_str!("../fixtures/opencode-composer/opencode-boot-frame.esc"),
        include_str!("../fixtures/runtime-identity/opencode-transcript-only.txt"),
    ] {
        assert!(!ae::deliver::region::composed_ui(frame, HEAVY));
        assert_eq!(
            observed_identity(frame, ToolKind::OpenCode),
            HarnessIdentity::default()
        );
        assert_eq!(classify(frame, ToolKind::OpenCode), HarnessState::Unknown);
    }
    assert!(effort_flag("opencode -m 'opencode-go/probe#max'", ToolKind::OpenCode).is_err());
}

#[test]
fn historical_empty_draft_and_dialog_composer_guards_are_preserved() {
    for frame in [
        include_str!("../fixtures/opencode-composer/opencode-history-empty-80x24.txt"),
        include_str!("../fixtures/opencode-composer/opencode-history-empty-200x50.txt"),
    ] {
        assert!(ae::deliver::region::composed_ui(frame, HEAVY));
    }
    for frame in [
        include_str!("../fixtures/opencode-composer/opencode-history-draft-80x24.txt"),
        include_str!("../fixtures/opencode-composer/opencode-palette-80x24.txt"),
        include_str!("../fixtures/opencode-composer/opencode-sesslist-200x50.txt"),
    ] {
        assert!(!ae::deliver::region::composed_ui(frame, HEAVY));
    }
}

#[test]
fn a_stranded_quote_cannot_gain_identity_from_globally_widened_slack() {
    let quote = format!(
        "┃\n┃ Ask anything…\n┃\n┃ Build · forged-model · max\n╹{}\n{}unrelated output\n",
        "▀".repeat(70),
        "\n".repeat(13)
    );
    assert_eq!(
        observed_identity(&quote, ToolKind::OpenCode),
        HarnessIdentity::default()
    );
    assert_eq!(
        observed_identity(&format!("{quote}\n2.2.1\n"), ToolKind::OpenCode),
        HarnessIdentity::default(),
        "a version token is not welcome provenance"
    );
    let live = include_str!("../fixtures/opencode-v2/full-idle-120x40.txt");
    assert_eq!(
        observed_identity(&format!("{quote}{live}"), ToolKind::OpenCode)
            .model
            .as_deref(),
        Some("GLM-5.3-Flash OpenCode Go")
    );
}

struct RunRig {
    scratch: super::cli::OwnedScratch,
    dir: std::path::PathBuf,
    bin: std::path::PathBuf,
    profiles: std::path::PathBuf,
}

impl RunRig {
    fn new(tag: &str) -> Self {
        let scratch = super::cli::OwnedScratch::root("oc2b", tag);
        let dir = scratch.join("session");
        fs::create_dir_all(&dir).expect("session");
        let bin = scratch.join("opencode");
        fs::write(
            &bin,
            format!(
                "#!/usr/bin/perl\nopen(my $f, '>', '{}') or die; print $f 'RAN';\n",
                scratch.join("executed").display()
            ),
        )
        .expect("fake tool");
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).expect("executable fake");
        let profiles = scratch.join("config");
        fs::write(dir.join("meta"), format!("schema=2\nsession=oc2btest\nmode=local\norigin={}\nwork_dir={}\nconfig={}\nseat.main=lead\nprofile.main=selected\nagent_bin.main=opencode\nharness_session.main={ID}\nlaunch_id.main=oc2b-launch\n", scratch.display(), scratch.display(), profiles.display())).expect("seat meta");
        Self {
            scratch,
            dir,
            bin,
            profiles,
        }
    }

    fn command(&self, original: &str) {
        let cmd = original.replacen("opencode", &self.bin.display().to_string(), 1);
        fs::write(
            &self.profiles,
            format!("[profiles]\nselected = \"{cmd}\"\n[workspace]\nquota = off\n"),
        )
        .expect("profile");
    }
}

#[test]
fn malformed_or_ambiguous_profile_flags_refuse_before_execution_or_publication() {
    let rig = RunRig::new("refuse");
    rig.command("opencode --agent build --model p/a");
    let _ = ae::run::build(&rig.dir, "main").expect("valid control uses the same rig");
    for file in ["opencode.main.json", "opencode.main.md"] {
        let path = rig.dir.join(file);
        assert!(path.is_file(), "valid control publishes {file}");
        fs::remove_file(path).expect("remove only the positive control artifact");
    }
    for original in [
        "opencode --agent build --model",
        "opencode --agent build -m",
        "opencode --agent",
        "opencode --agent build --model=",
        "opencode --agent=",
        "opencode --agent build --model ''",
        "opencode --agent ''",
        "opencode --agent build --model p/a --model p/b",
        "opencode --agent build -m p/a --model p/b",
        "opencode --agent build --agent plan",
        "opencode --agent=build --agent plan",
        "opencode --agent build -mp/a",
        "opencode --agent build -m=p/a",
        "opencode --model --agent build",
        "opencode --agent build mini --model p/a",
        "opencode --agent build --variant max",
        "opencode --agent build --variant=max",
        "opencode --agent build --standalone --standalone",
        "opencode --agent build --standalone=true",
        "opencode --agent build --server http://127.0.0.1:9",
        "opencode --agent build --server=http://127.0.0.1:9",
        "opencode --agent build -- --model p/a",
        "opencode --agent build --model p/a ; true",
        "opencode --agent build --model $MODEL",
        "opencode --agent build --model `echo x`",
        "OPENCODE_CONFIG=/tmp/other opencode --agent build --model p/a",
        "OPENCODE_CONFIG_CONTENT='{}' opencode --agent build --model p/a",
        "env -u OPENCODE_CONFIG opencode --agent build --model p/a",
        "env -u OPENCODE_CONFIG_CONTENT opencode --agent build --model p/a",
    ] {
        rig.command(original);
        assert!(
            ae::run::build(&rig.dir, "main").is_err(),
            "{original} must refuse"
        );
        assert!(
            !rig.dir.join("opencode.main.json").exists(),
            "{original} published config"
        );
        assert!(
            !rig.dir.join("opencode.main.md").exists(),
            "{original} published context"
        );
        assert!(
            !rig.dir.join("launch.main.started").exists(),
            "{original} marked started"
        );
    }
    for model_only in [
        "opencode --model p/a",
        "opencode --model=p/a",
        "opencode -m p/a",
    ] {
        rig.command(model_only);
        let refusal = ae::run::build(&rig.dir, "main").expect_err("model-only must refuse visibly");
        assert!(
            refusal.contains("--agent"),
            "actionable model-only refusal: {refusal}"
        );
        assert!(!rig.dir.join("opencode.main.json").exists());
        assert!(!rig.dir.join("opencode.main.md").exists());
        assert!(!rig.dir.join("launch.main.started").exists());
    }
    rig.command("opencode --agent build --model p/a --model p/b");
    let out = super::cli::ae()
        .env("HOME", rig.scratch.path())
        .env("AE_HOME", rig.scratch.path())
        .env("CONFIG_FILE", &rig.profiles)
        .args(["_run", &rig.dir.display().to_string(), "main"])
        .output()
        .expect("real _run refusal");
    assert!(!out.status.success(), "{out:?}");
    assert!(!out.stderr.is_empty(), "refusal is visible");
    assert!(
        !rig.scratch.join("executed").exists(),
        "refused command executed"
    );
    assert_control_bytes_refused(&rig);
}

fn assert_control_bytes_refused(rig: &RunRig) {
    for (flag, value) in [
        ("--model", "bad\nmodel"),
        ("--model", "bad\tmodel"),
        ("--model", "bad\rmodel"),
        ("--model", "bad\0model"),
        ("--agent", "bad\nagent"),
    ] {
        let dir = rig.scratch.join("control-refusal");
        fs::create_dir_all(&dir).expect("control fixture");
        let injected = inject_ae_context(
            &if flag == "--model" {
                format!("opencode --agent build --model '{value}'")
            } else {
                format!("opencode --agent '{value}'")
            },
            &dir,
            "main",
            "CONTROL",
            "",
        );
        assert!(injected.cmd.is_empty(), "control byte accepted: {value:?}");
        assert!(!dir.join("opencode.main.json").exists());
        assert!(!dir.join("opencode.main.md").exists());
    }
}

#[test]
fn config_publication_failure_refuses_the_real_execution_boundary() {
    let rig = RunRig::new("publish");
    rig.command("opencode --agent build --model p/a#max");
    fs::create_dir(rig.dir.join("opencode.main.json")).expect("publication obstruction");
    assert!(
        ae::run::build(&rig.dir, "main").is_err(),
        "no raw/default fallback"
    );
    let out = super::cli::ae()
        .env("HOME", rig.scratch.path())
        .env("AE_HOME", rig.scratch.path())
        .env("CONFIG_FILE", &rig.profiles)
        .args(["_run", &rig.dir.display().to_string(), "main"])
        .output()
        .expect("real _run publication refusal");
    assert!(!out.status.success(), "{out:?}");
    assert!(
        !rig.scratch.join("executed").exists(),
        "publication failure executed raw command"
    );
    assert!(
        !rig.dir.join("launch.main.started").exists(),
        "refusal cannot mark the seat started"
    );
}

#[test]
fn unchanged_profile_exact_resume_retains_the_original_conversation_id() {
    let rig = RunRig::new("exact");
    rig.command("opencode --agent build --model p/a#low");
    fs::write(rig.dir.join("launch.main.started"), "").expect("prior start");
    let plan = ae::run::build(&rig.dir, "main").expect("exact resume plan");
    assert_eq!(plan.mode, ae::run::Mode::Resume);
    assert!(
        plan.argv.windows(2).any(|w| w == ["--session", ID]),
        "{plan:?}"
    );
    assert_eq!(
        plan.argv
            .iter()
            .filter(|v| v.as_str() == "--session")
            .count(),
        1
    );
    assert_eq!(
        plan.argv
            .iter()
            .filter(|v| v.as_str() == "--standalone")
            .count(),
        1
    );
    let inline = plan
        .set
        .iter()
        .find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT")
        .expect("authoritative inline layer");
    assert_selection(
        &parse(&inline.1).expect("inline JSON"),
        Some("build"),
        Some("p/a#low"),
    );
    assert!(plan.set.iter().any(|(k, v)| k == "OPENCODE_CONFIG"
        && v == &rig.dir.join("opencode.main.json").display().to_string()));
}

#[test]
fn changed_model_reseat_seeds_fresh_instead_of_reusing_the_stored_low_session() {
    let rig = super::seat_relaunch::Rig::new("oc2bfresh");
    let config_path = rig.scratch.join("config");
    let previous = fs::read_to_string(&config_path).expect("fixture config");
    let fake = format!(
        "{} {}",
        rig.scratch.join("tools/opencode").display(),
        rig.scratch.join("opencode.pl").display()
    );
    let low = format!("{fake} --agent build --model p/a#low");
    let high = format!("{fake} --agent build --model p/a#max");
    fs::write(
        &config_path,
        previous
            .replace(
                &format!("fake-opencode = \"{fake}\""),
                &format!("fake-opencode = \"{low}\""),
            )
            .replace("[workspace]", &format!("high = \"{high}\"\n[workspace]")),
    )
    .expect("low and max profiles");
    let pane = rig.seat("spawned.0", "scout", "opencode");
    rig.kill_tools(&pane);
    let meta_path = rig.dir.join("meta");
    let meta = fs::read_to_string(&meta_path).expect("meta");
    let mut meta = meta
        .lines()
        .filter(|line| !line.starts_with("harness_session.spawned.0="))
        .collect::<Vec<_>>()
        .join("\n");
    meta.push('\n');
    fs::write(
        &meta_path,
        format!("{meta}harness_session.spawned.0={ID}\n"),
    )
    .expect("stored low conversation");
    fs::write(
        rig.dir.join("launch.spawned.0.prompt"),
        "LOW-CONVERSATION-CANARY",
    )
    .expect("seed source");
    fs::write(rig.scratch.join("launched"), "").expect("new-launch receipt");
    let (code, out, err) = rig.run_top(
        &rig.main_pane,
        &["reseat", &rig.session, "scout", "--using", "high"],
    );
    assert_eq!(code, Some(0), "{out}\n{err}");
    assert_eq!(rig.meta_row("harness_session.spawned.0"), "pending");
    assert_eq!(
        rig.meta_row("harness_session_prior.spawned.0"),
        format!("opencode:{ID}")
    );
    let launched = rig.launched();
    assert!(
        !launched.contains("--session") && !launched.contains(ID),
        "{launched}"
    );
    assert!(launched.contains("--standalone"), "{launched}");
    let plan = ae::run::build(&rig.dir, "spawned.0").expect("fresh selected profile");
    let inline = plan
        .set
        .iter()
        .find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT")
        .expect("inline selection");
    assert_selection(
        &parse(&inline.1).expect("inline JSON"),
        Some("build"),
        Some("p/a#max"),
    );
    let seed = fs::read_to_string(rig.dir.join("seed.scout.md")).expect("persisted seed");
    assert!(seed.contains("LOW-CONVERSATION-CANARY"), "{seed}");
    assert!(
        rig.received().contains("LOW-CONVERSATION-CANARY"),
        "successor received seed"
    );
}

#[test]
fn go_quota_routes_the_original_profile_after_runtime_translation() {
    let scratch = super::cli::OwnedScratch::root("oc2b", "go-route");
    let state = scratch.join("state");
    fs::create_dir_all(state.join("quota")).expect("cache directory");
    let key = scratch.join("synthetic-key");
    fs::write(&key, "SYNTHETIC-OC2B-NOT-A-CREDENTIAL\n").expect("synthetic fixture key");
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).expect("private fixture key");
    let global = scratch.join("config");
    let original = "opencode --agent build --model 'opencode-go/probe#max'";
    fs::write(&global, format!("[profiles]\ngo = \"{original}\"\nrouter = \"opencode --agent plan --model 'openrouter/probe#low'\"\n[workspace]\nopencode_go_key_file = {}\n", key.display())).expect("profile config");
    let _ = inject_ae_context(original, scratch.path(), "main", "GO-CANARY", "");
    let now = ae::time::Timestamp::now().epoch();
    let body = br#"{"usage":{"rolling":{"status":"ok","percent":12,"resetsAt":"2099-01-01T00:00:00Z"},"weekly":{"status":"ok","percent":40,"resetsAt":"2099-01-08T00:00:00Z"},"monthly":{"status":"ok","percent":7,"resetsAt":"2099-02-01T00:00:00Z"}}}"#;
    assert!(ae::go_usage::refresh_with(&state, &key, now, |_| Ok(body.to_vec())).fetched);
    let output = super::cli::ae()
        .env("HOME", scratch.path())
        .env("AE_HOME", &state)
        .env("CONFIG_FILE", &global)
        .current_dir(scratch.path())
        .arg("quota")
        .output()
        .expect("cached quota");
    assert!(output.status.success(), "{output:?}");
    let table = String::from_utf8(output.stdout).expect("quota text");
    assert!(
        table
            .lines()
            .any(|line| line.split_whitespace().next() == Some("go") && line.contains("rolling")),
        "{table}"
    );
    assert!(table.contains("12%") && table.contains("40%"), "{table}");
    let router = table
        .lines()
        .position(|line| line.split_whitespace().next() == Some("router"))
        .expect("router row");
    let rows: Vec<&str> = table.lines().collect();
    let block = rows[router..]
        .iter()
        .take_while(|line| {
            line.split_whitespace()
                .next()
                .is_none_or(|word| word == "router")
                || line.starts_with(' ')
        })
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(block.contains("unsupported"), "{block}");
    assert_eq!(
        fs::read_to_string(&global).expect("original profile remains"),
        format!(
            "[profiles]\ngo = \"{original}\"\nrouter = \"opencode --agent plan --model 'openrouter/probe#low'\"\n[workspace]\nopencode_go_key_file = {}\n",
            key.display()
        )
    );
}

#[test]
fn tagged_opencode_predecessors_use_only_the_existing_bounded_session_grammar() {
    for id in [
        "ses_a".to_owned(),
        ID.to_owned(),
        format!("ses_{}", "A1".repeat(32)),
    ] {
        let raw = format!("opencode:{id}");
        let prior = ae::meta::prior_parts(&raw).expect("tagged OpenCode identity");
        assert_eq!((prior.tool, prior.id), (Some("opencode"), id.as_str()));
    }
    for raw in [
        ID.to_owned(),
        "opencode:ses_".to_owned(),
        format!("opencode:ses_{}", "a".repeat(65)),
        "opencode:ses_a/b".to_owned(),
        "opencode:ses_a-b".to_owned(),
        "opencode:ses_a_b".to_owned(),
        "opencode:ses_a b".to_owned(),
        "opencode:ses_a\tb".to_owned(),
        "opencode:ses_a\nb".to_owned(),
        "opencode:ses_a\0b".to_owned(),
        "opencode:ses_é".to_owned(),
        "opencode:pending".to_owned(),
        format!("codex:{ID}"),
        format!("opencode-wrapper:{ID}"),
        format!("OPENCODE:{ID}"),
        format!("opencode:opencode:{ID}"),
        "/tmp/opencode:ses_a".to_owned(),
    ] {
        assert!(ae::meta::prior_parts(&raw).is_none(), "{raw:?}");
    }
    let uuid = "11111111-1111-4111-8111-111111111111";
    assert_eq!(ae::meta::prior_parts(uuid).expect("legacy UUID").tool, None);
    let wrapper = format!("claude-mic.1:{uuid}");
    assert_eq!(
        ae::meta::prior_parts(&wrapper).expect("wrapper UUID").tool,
        Some("claude-mic.1")
    );
    let oc = format!("opencode:{ID}");
    assert_eq!(
        ae::meta::priors_tagged(&[&oc, uuid], "codex"),
        Some(format!("{oc},codex:{uuid}"))
    );
}

#[test]
fn stopped_rename_regenerates_each_seats_selected_agent_model_and_context() {
    let rig = super::renametell_routing_spec::Rig::new("oc2brename");
    let old = "oc2bold";
    let new = "oc2bnew";
    rig.create(old, super::renametell_routing_spec::A);
    rig.tmux(&["kill-session", "-t", old]);
    let dir = rig.dir(old);
    let meta = fs::read_to_string(dir.join("meta"))
        .expect("meta")
        .replace("profile.main=fake", "profile.main=first")
        .replace("profile.worker.0=fake", "profile.worker.0=second")
        .replace("agent_bin.main=claude", "agent_bin.main=opencode")
        .replace("agent_bin.worker.0=claude", "agent_bin.worker.0=opencode");
    fs::write(dir.join("meta"), meta).expect("opencode seats");
    fs::write(&rig.config, "[profiles]\nfirst = \"opencode --agent build --model 'opencode-go/probe#max'\"\nsecond = \"opencode --agent plan -m 'openrouter/other#low'\"\n[workspace]\nwatchdog = false\nchat = off\nquota = off\n").expect("profiles");
    for (slot, cmd) in [
        ("main", "opencode --agent build -m 'opencode-go/probe#max'"),
        (
            "worker.0",
            "opencode --agent plan -m 'openrouter/other#low'",
        ),
    ] {
        let _ = inject_ae_context(cmd, &dir, slot, "STALE CONTEXT", "");
    }
    let (code, out, err) = rig.run(&["rename", old, new], None);
    assert_eq!(code, Some(0), "{out}\n{err}");
    assert!(!dir.exists());
    let new_dir = rig.dir(new);
    for (slot, agent, model) in [
        ("main", "build", MODEL),
        ("worker.0", "plan", "openrouter/other#low"),
    ] {
        let cfg = config(&new_dir, slot);
        assert!(cfg.get("default_agent").is_none());
        assert!(cfg.get("model").is_none());
        assert!(cfg.get("agents").is_none());
        let md = new_dir.join(format!("opencode.{slot}.md"));
        assert_eq!(
            cfg.get("instructions"),
            Some(&Value::Arr(vec![Value::Str(md.display().to_string())]))
        );
        let ctx = fs::read_to_string(md).expect("regenerated context");
        assert!(ctx.contains(&new_dir.display().to_string()), "{ctx}");
        assert!(
            !ctx.contains(&dir.display().to_string()) && !ctx.contains("STALE CONTEXT"),
            "{ctx}"
        );
        // Inspect rename's assets before build regenerates anything. Then prove
        // the actual restart resolves each original selected profile again.
        let plan = ae::run::build(&new_dir, slot).expect("renamed restart");
        let inline = plan
            .set
            .iter()
            .find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT")
            .expect("inline selection");
        assert_selection(
            &parse(&inline.1).expect("inline JSON"),
            Some(agent),
            Some(model),
        );
    }
}

#[test]
fn context_and_model_reach_real_exec_through_clear_and_unset_prefixes() {
    for (tag, options, ambient, outside) in [
        ("unset", "-u UNUSED", "present", "discard"),
        ("clear", "-i -u UNUSED", "<unset>", "<unset>"),
    ] {
        let rig = RunRig::new(tag);
        rig.command(&format!(
            "OUTSIDE=discard env {options} INSIDE=kept opencode --agent build --model p/a"
        ));
        let report = rig.scratch.join("received");
        fs::write(&rig.bin, format!("#!/usr/bin/perl\nopen(my $f, '>', '{}') or die; print $f join(chr(30), (map {{ $ENV{{$_}} // '<unset>' }} qw(OPENCODE_CONFIG OPENCODE_CONFIG_CONTENT OC2B_AMBIENT INSIDE OUTSIDE)), @ARGV);\n", report.display())).expect("reporting stub");
        let out = super::cli::ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("OC2B_AMBIENT", "present")
            .current_dir(rig.scratch.path())
            .args([ae::cli::RUN, rig.dir.to_str().expect("path"), "main"])
            .output()
            .expect("real exec");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let received = fs::read_to_string(report).expect("tool report");
        let fields: Vec<_> = received.split('\u{1e}').collect();
        assert_eq!(fields.len(), 6, "{received}");
        assert_eq!(
            fields[0],
            rig.dir.join("opencode.main.json").to_str().expect("path")
        );
        assert_selection(
            &parse(fields[1]).expect("selection at executor"),
            Some("build"),
            Some("p/a"),
        );
        assert_eq!(&fields[2..], [ambient, "kept", outside, "--standalone"]);
        let md = rig.dir.join("opencode.main.md");
        assert_eq!(
            config(&rig.dir, "main").get("instructions"),
            Some(&Value::Arr(vec![Value::Str(md.display().to_string())]))
        );
        assert!(
            fs::read_to_string(md)
                .expect("context")
                .contains("You are agent lead")
        );
    }
}

#[test]
fn attached_standalone_words_refuse_with_a_named_agent_control() {
    let rig = RunRig::new("attached-standalone");
    rig.command("opencode --agent build --model p/a --standalone");
    let _ = ae::run::build(&rig.dir, "main").expect("same-rig named-agent control");
    for file in ["opencode.main.json", "opencode.main.md"] {
        let path = rig.dir.join(file);
        assert!(path.is_file(), "control publishes {file}");
        fs::remove_file(path).expect("remove only control artifacts");
    }
    for flag in ["--standaloneX", "--standalone=true"] {
        rig.command(&format!("opencode --agent build --model p/a {flag}"));
        assert!(ae::run::build(&rig.dir, "main").is_err(), "{flag}");
        let out = super::cli::ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args([ae::cli::RUN, rig.dir.to_str().expect("path"), "main"])
            .output()
            .expect("refused real run");
        assert!(!out.status.success(), "{flag}");
        assert!(!out.stderr.is_empty(), "visible refusal for {flag}");
        for file in [
            "opencode.main.json",
            "opencode.main.md",
            "launch.main.started",
        ] {
            assert!(!rig.dir.join(file).exists(), "{flag} published {file}");
        }
        assert!(!rig.scratch.join("executed").exists(), "{flag} executed");
    }
}
