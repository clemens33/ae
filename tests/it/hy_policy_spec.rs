//! Frozen acceptance for hygiene decision 3, authored before implementation.
//! Requirements and mutation witnesses: .local/hy-policy/spec/acceptance.md.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures read and write only process-owned scratch state"
)]

use std::path::{Path, PathBuf};

use ae::config::{ConfigError, parse_identity};
use ae::render::{context_document, manifest_document};

const SOLO: &str = "mode=local\nseat.main=lead\n";
const PAIR: &str = "mode=local\nlayout=lead-pair\nschema=2\nseat.main=lead\nseat.worker.0=colead\n";
const COMMON: &str = "ALL-SEATS-7d4885: keep this unchanged.";
const POLICY: &str = "SPAWN-POLICY-7d4885: choose only approved profiles.\nSecond policy line: ${sess} $HOME # literal.";

struct Rig(super::cli::OwnedScratch);

impl Rig {
    fn new(tag: &str, meta: &str) -> Self {
        let rig = Self(super::cli::OwnedScratch::root("hpol", tag));
        let private = format!(
            "{meta}tmux_server_kind=socket\ntmux_server={}\n",
            rig.dir().join("unused.sock").display()
        );
        std::fs::write(rig.dir().join("meta"), private).expect("scratch meta");
        rig
    }

    fn dir(&self) -> &Path {
        self.0.path()
    }

    fn config(&self, name: &str, text: &str) -> PathBuf {
        let file = self.dir().join(name);
        std::fs::write(&file, text).expect("scratch config");
        file
    }

    fn context(&self, slot: &str, files: &[PathBuf]) -> String {
        context_document(self.dir(), "s", "/w", slot, files)
    }

    fn workspace(&self, files: &[PathBuf]) -> String {
        manifest_document(self.dir(), "s", "/w", "/o", "local", "%0", files)
    }
}

fn config_with_policy(policy: &str) -> String {
    format!("[prompt]\ninstructions = {COMMON}\nspawn_instructions = \"\"\"\n{policy}\n\"\"\"\n")
}

fn assert_full(document: &str, policy: &str) {
    assert!(document.contains(policy), "full policy missing: {document}");
    assert_eq!(document.matches(policy).count(), 1, "policy duplicated");
}

fn assert_policy_absent(document: &str, policy: &str) {
    for body_line in policy.lines().filter(|line| !line.is_empty()) {
        assert!(
            !document.contains(body_line),
            "policy body line leaked into worker context: {document}"
        );
    }
}

fn assert_pointer(document: &str, baseline: &str, dir: &Path) {
    let suffix = document
        .strip_prefix(baseline)
        .expect("scoped policy must preserve the complete prior context prefix");
    assert!(!suffix.trim().is_empty(), "worker needs a policy pointer");
    assert_eq!(suffix.trim().lines().count(), 1, "one pointer line");
    assert!(
        suffix.contains(&dir.join("workspace.md").display().to_string()),
        "pointer must name its actual session reference: {suffix}"
    );
    assert!(
        suffix.contains("Spawn instructions"),
        "section name: {suffix}"
    );
    assert!(
        suffix.to_lowercase().contains("before any spawn"),
        "pointer must require the read before profile selection/spawn: {suffix}"
    );
    assert_policy_absent(document, POLICY);
}

#[test]
fn scoped_policy_lead_pair_and_solo_lead_receive_full_text() {
    for (tag, meta, slots) in [
        ("pair", PAIR, &["main", "worker.0"][..]),
        ("solo", SOLO, &["main"][..]),
    ] {
        let rig = Rig::new(tag, meta);
        let config = rig.config("global", &config_with_policy(POLICY));
        for slot in slots {
            let document = rig.context(slot, std::slice::from_ref(&config));
            assert_full(&document, POLICY);
            assert_full(&document, COMMON);
        }
    }
}

#[test]
fn scoped_policy_workers_receive_one_resolving_pointer_and_no_policy_body() {
    for (tag, meta, slots) in [
        ("pair-w", PAIR, &["spawned.7", "worker.1"][..]),
        ("solo-w", SOLO, &["worker.0", "spawned.0"][..]),
        (
            "no-peer",
            "mode=local\nlayout=lead-pair\nseat.main=lead\n",
            &["worker.0"][..],
        ),
    ] {
        let rig = Rig::new(tag, meta);
        let plain = rig.config("plain", &format!("[prompt]\ninstructions = {COMMON}\n"));
        let scoped = rig.config("scoped", &config_with_policy(POLICY));
        let workspace = rig.workspace(std::slice::from_ref(&scoped));
        assert!(
            workspace.ends_with(&format!("\n## Spawn instructions\n\n{POLICY}\n")),
            "pointer section must contain literal complete policy: {workspace}"
        );
        for slot in slots {
            let baseline = rig.context(slot, std::slice::from_ref(&plain));
            let document = rig.context(slot, std::slice::from_ref(&scoped));
            assert_pointer(&document, &baseline, rig.dir());
            assert_full(&document, COMMON);
        }
    }
}

#[test]
fn scoped_policy_slotless_unknown_and_untrusted_identity_keep_full_text() {
    let rig = Rig::new("other", "mode=local\nseat.main=bad/identity\n");
    let config = rig.config("global", &config_with_policy(POLICY));
    for slot in ["", "mystery", "workers.1", "main"] {
        let document = rig.context(slot, std::slice::from_ref(&config));
        assert_full(&document, POLICY);
        assert_full(&document, COMMON);
        assert!(!document.contains("You are agent bad/identity"));
    }
}

#[test]
fn scoped_policy_absent_key_preserves_existing_context_goldens_and_legacy_suffix() {
    for (tag, meta, slot, config, golden) in [
        (
            "gold-s",
            SOLO.to_owned(),
            "main",
            "",
            include_str!("../fixtures/launch-context-solo-main-aware.golden"),
        ),
        (
            "gold-p",
            PAIR.to_owned(),
            "main",
            "[workspace]\nquota = off\n",
            include_str!("../fixtures/launch-context-pair-main-unaware.golden"),
        ),
        (
            "gold-w",
            format!("{SOLO}seat.spawned.4=builder\n"),
            "spawned.4",
            "",
            include_str!("../fixtures/launch-context-spawned-worker.golden"),
        ),
    ] {
        let rig = Rig::new(tag, &meta);
        let plain = rig.config("plain", config);
        let baseline = rig.context(slot, &[plain]);
        assert_eq!(
            baseline.replace(&rig.dir().display().to_string(), "<DIR>"),
            golden
        );
        let common = rig.config(
            "common",
            &format!("{config}\n[prompt]\ninstructions = {COMMON}\n"),
        );
        assert_eq!(
            rig.context(slot, &[common]),
            format!("{baseline} --- Workspace instructions: {COMMON}")
        );
    }
}

#[test]
fn scoped_policy_legacy_instructions_still_reach_every_class_verbatim() {
    let rig = Rig::new("legacy", PAIR);
    let literal = "first ${sess} # literal\n  second line $HOME";
    let config = rig.config(
        "global",
        &format!("[prompt]\ninstructions = \"\"\"\n{literal}\n\"\"\"\n"),
    );
    for slot in ["main", "worker.0", "worker.1", "spawned.9", "", "other"] {
        let document = rig.context(slot, std::slice::from_ref(&config));
        assert!(document.ends_with(&format!(" --- Workspace instructions: {literal}")));
        assert!(!document.contains("Spawn instructions"));
    }
    assert!(!rig.workspace(&[config]).contains("## Spawn instructions"));
}

#[test]
fn scoped_policy_both_keys_resolve_independently_and_empty_local_masks_only_its_key() {
    let rig = Rig::new("mask", PAIR);
    let global = rig.config(
        "global",
        "[prompt]\ninstructions = GLOBAL-COMMON\nspawn_instructions = GLOBAL-POLICY\n",
    );
    for (tag, local, common, policy) in [
        (
            "none",
            "[workspace]\nquota = off\n",
            Some("GLOBAL-COMMON"),
            Some("GLOBAL-POLICY"),
        ),
        (
            "i",
            "[prompt]\ninstructions = LOCAL-COMMON\n",
            Some("LOCAL-COMMON"),
            Some("GLOBAL-POLICY"),
        ),
        (
            "s",
            "[prompt]\nspawn_instructions = LOCAL-POLICY\n",
            Some("GLOBAL-COMMON"),
            Some("LOCAL-POLICY"),
        ),
        (
            "empty-i",
            "[prompt]\ninstructions = \"\"\n",
            None,
            Some("GLOBAL-POLICY"),
        ),
        (
            "empty-s",
            "[prompt]\nspawn_instructions = \"\"\n",
            Some("GLOBAL-COMMON"),
            None,
        ),
        (
            "empty-b",
            "[prompt]\nspawn_instructions = \"\"\"\n\"\"\"\n",
            Some("GLOBAL-COMMON"),
            None,
        ),
        (
            "last",
            "[prompt]\nspawn_instructions = FIRST-POLICY\nspawn_instructions = LOCAL-POLICY\n",
            Some("GLOBAL-COMMON"),
            Some("LOCAL-POLICY"),
        ),
        (
            "last-empty",
            "[prompt]\nspawn_instructions = LOCAL-POLICY\nspawn_instructions = \"\"\n",
            Some("GLOBAL-COMMON"),
            None,
        ),
    ] {
        let local = rig.config(tag, local);
        let files = [global.clone(), local];
        let workspace = rig.workspace(&files);
        for slot in ["main", "worker.0", "spawned.0", ""] {
            let document = rig.context(slot, &files);
            match common {
                Some(text) => assert_full(&document, text),
                None => assert!(!document.contains("GLOBAL-COMMON")),
            }
            match policy {
                Some(text) if slot != "spawned.0" => assert_full(&document, text),
                Some(_) => {
                    assert!(document.contains("Spawn instructions"));
                    assert!(!document.contains("GLOBAL-POLICY"));
                    assert!(!document.contains("LOCAL-POLICY"));
                }
                None => assert!(!document.contains("Spawn instructions")),
            }
        }
        match policy {
            Some(text) => {
                assert!(workspace.ends_with(&format!("\n## Spawn instructions\n\n{text}\n")));
                assert!(!workspace.contains("FIRST-POLICY"));
            }
            None => assert!(!workspace.contains("## Spawn instructions")),
        }
    }
}

#[test]
fn scoped_policy_new_key_alone_still_gives_workers_a_pointer() {
    let rig = Rig::new("only", SOLO);
    let config = rig.config("global", "[prompt]\nspawn_instructions = SOLE-POLICY\n");
    let document = rig.context("spawned.0", std::slice::from_ref(&config));
    assert_pointer(&document, &rig.context("spawned.0", &[]), rig.dir());
    assert!(!document.contains("SOLE-POLICY"));
    assert!(
        rig.workspace(&[config])
            .contains("\n## Spawn instructions\n\nSOLE-POLICY\n")
    );
}

#[test]
fn scoped_policy_no_final_scalar_newline_is_dropped_but_block_closer_at_eof_is_kept() {
    let rig = Rig::new("eof", SOLO);
    let global = rig.config("global", "[prompt]\nspawn_instructions = GLOBAL-POLICY\n");
    let scalar = rig.config("scalar", "[prompt]\nspawn_instructions = LOST-POLICY");
    let document = rig.context("main", &[global.clone(), scalar]);
    assert_full(&document, "GLOBAL-POLICY");
    assert!(!document.contains("LOST-POLICY"));
    let block = rig.config(
        "block",
        "[prompt]\r\nspawn_instructions = \"\"\"\r\none\r\ntwo\r\n\"\"\"",
    );
    let document = rig.context("main", &[global, block]);
    assert_full(&document, "one\ntwo");
    assert!(!document.contains("GLOBAL-POLICY"));
}

#[test]
fn scoped_policy_block_body_is_inert_and_both_block_keys_keep_their_own_bytes() {
    let policy = "[clients]\ninvalid/name = bad\n# policy comment\n   \"\"\"\ntext \"\"\" in the middle\n${sess} $HOME";
    let text = format!(
        "[prompt]\ninstructions = \"\"\"\n{COMMON}\n\"\"\"\nspawn_instructions = \"\"\"  \n{policy}\n\"\"\"\t\n[workspace]\nmain = lead\n"
    );
    let parsed = parse_identity(&text).expect("new block must parse and hide its body from INI");
    assert!(parsed.clients.is_empty());
    assert_eq!(parsed.main.as_deref(), Some("lead"));
    let rig = Rig::new("inert", SOLO);
    let config = rig.config("global", &text);
    let document = rig.context("main", std::slice::from_ref(&config));
    assert_full(&document, COMMON);
    assert_full(&document, policy);
    assert!(rig.workspace(&[config]).contains(policy));
}

#[test]
fn scoped_policy_blocks_remain_limited_to_exactly_the_two_prompt_keys() {
    for (text, key, line) in [
        (
            "spawn_instructions = \"\"\"\n\"\"\"\n",
            "spawn_instructions",
            1,
        ),
        (
            "[workspace]\nspawn_instructions = \"\"\"\n\"\"\"\n",
            "workspace.spawn_instructions",
            2,
        ),
        ("[prompt]\nnotes = \"\"\"\n\"\"\"\n", "prompt.notes", 2),
        (
            "[prompt]\nSpawn_instructions = \"\"\"\n\"\"\"\n",
            "prompt.Spawn_instructions",
            2,
        ),
    ] {
        let err = parse_identity(text).expect_err("no other key gains block syntax");
        assert!(
            matches!(&err, ConfigError::InstructionBlock { key: named, line: at, .. } if named == key && *at == line),
            "wrong refusal: {err}"
        );
        assert!(err.to_string().contains("prompt.spawn_instructions"));
    }
    for text in [
        "[prompt]\nspawn_instructions = \"\"\"\nnever closed\n",
        "[prompt]\nspawn_instructions = \"\"\"\n   \"\"\"\n",
    ] {
        let err = parse_identity(text).expect_err("indented triple quotes cannot close the block");
        assert!(matches!(
            err,
            ConfigError::UnterminatedInstructionBlock { line: 2, .. }
        ));
    }
}

fn argv_context(plan: &ae::run::Plan) -> &str {
    let at = plan
        .argv
        .iter()
        .position(|word| word == "--append-system-prompt")
        .expect("fixture claude context channel");
    plan.argv.get(at + 1).expect("actual context argument")
}

fn bootstrap_live_reference(rig: &super::seat_relaunch::Rig) -> (PathBuf, String) {
    let config = rig.scratch.join("config");
    let original = std::fs::read_to_string(&config).expect("fixture config");
    let workspace = canonical_live_manifest(rig);
    assert!(!workspace.contains("## Spawn instructions"));
    std::fs::write(rig.dir.join("workspace.md"), workspace).expect("initial launch reference");
    (config, original)
}

fn canonical_live_manifest(rig: &super::seat_relaunch::Rig) -> String {
    manifest_document(
        &rig.dir,
        &rig.meta_row("session"),
        &rig.meta_row("work_dir"),
        &rig.meta_row("origin"),
        &rig.meta_row("mode"),
        &rig.meta_row("main_pane"),
        &[PathBuf::from(rig.meta_row("config"))],
    )
}

fn assert_live_reference(rig: &super::seat_relaunch::Rig, policy: &str) {
    let workspace = std::fs::read_to_string(rig.dir.join("workspace.md"))
        .expect("live operation must publish the pointer target");
    assert_eq!(
        workspace,
        canonical_live_manifest(rig),
        "refresh must retain canonical session facts, roster and all document sections"
    );
    assert!(
        workspace.ends_with(&format!("\n## Spawn instructions\n\n{policy}\n")),
        "live operation left a missing/stale policy section: {workspace}"
    );
}

#[test]
fn scoped_policy_create_resume_and_real_reseat_use_the_same_role_scoping() {
    let rig = super::seat_relaunch::Rig::new("hpol-life");
    let (config, original) = bootstrap_live_reference(&rig);
    std::fs::write(&config, format!("{original}{}", config_with_policy(POLICY)))
        .expect("policy added after initial launch");

    // CREATE and RESUME are actual run plans, with the marker selecting mode.
    rig.seat_rows("spawned.0", "scout", "claude", "claude");
    let create = ae::run::build(&rig.dir, "spawned.0").expect("create plan");
    assert_eq!(create.mode, ae::run::Mode::Create);
    let expected = context_document(
        &rig.dir,
        &rig.session,
        &rig.scratch.join("work").display().to_string(),
        "spawned.0",
        std::slice::from_ref(&config),
    );
    assert_eq!(argv_context(&create), expected);
    assert!(expected.contains("Spawn instructions"));
    assert_policy_absent(&expected, POLICY);
    assert_full(&expected, COMMON);
    std::fs::write(ae::run::started_marker(&rig.dir, "spawned.0"), "").expect("resume marker");
    let resume = ae::run::build(&rig.dir, "spawned.0").expect("resume plan");
    assert_eq!(resume.mode, ae::run::Mode::Resume);
    let resumed = argv_context(&resume);
    assert!(
        resumed.starts_with(&expected),
        "same scoped context on resume"
    );
    assert_policy_absent(resumed, POLICY);

    // A real resumed _run must update the old reference BEFORE the tool starts.
    let pane = rig.new_pane("spawned.0", "scout");
    rig.start(&pane, "spawned.0", "claude");
    assert_live_reference(&rig, POLICY);
    assert_policy_absent(&rig.launched(), POLICY);
    rig.kill_tools(&pane);

    // Editing the policy again must be observed by the real relaunch path.
    let relaunched_policy = "RELAUNCH-POLICY-7d4885\nSecond relaunch policy line.";
    std::fs::write(
        &config,
        format!("{original}{}", config_with_policy(relaunched_policy)),
    )
    .expect("second live policy edit");
    let (code, out, err) = rig.run("_relaunch", &["scout"]);
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert_live_reference(&rig, relaunched_policy);
    assert_policy_absent(&rig.launched(), relaunched_policy);
    rig.kill_tools(&pane);

    // Real reseat keeps the same worker slot and publishes the latest policy.
    let reseated_policy = "RESEAT-POLICY-7d4885\nSecond reseat policy line.";
    std::fs::write(
        &config,
        format!("{original}{}", config_with_policy(reseated_policy)),
    )
    .expect("third live policy edit");
    let (code, out, err) = rig.run_top(
        &rig.main_pane,
        &["reseat", &rig.session, "scout", "--using", "fake-opencode"],
    );
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert_eq!(rig.meta_row("seat.spawned.0"), "scout");
    assert_live_reference(&rig, reseated_policy);
    let received = std::fs::read_to_string(rig.dir.join("opencode.spawned.0.md"))
        .expect("successor context channel published");
    assert!(
        received.contains(&expected),
        "successor context channel must contain identical scoped context: {received}"
    );
    assert_policy_absent(&received, reseated_policy);
    assert_policy_absent(&rig.received(), reseated_policy);
}

#[test]
fn scoped_policy_real_spawn_refreshes_reference_after_a_live_config_migration() {
    let rig = super::seat_relaunch::Rig::new("hpol-spawn");
    let (config, original) = bootstrap_live_reference(&rig);
    std::fs::write(&config, format!("{original}{}", config_with_policy(POLICY)))
        .expect("policy migrated after initial launch");
    let (code, out, err) = rig.run(
        ae::cli::SPAWN,
        &[
            "scout",
            "--using",
            "fake-claude",
            "--",
            "bounded fixture task",
        ],
    );
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert_live_reference(&rig, POLICY);
    let expected = context_document(
        &rig.dir,
        &rig.session,
        &rig.scratch.join("work").display().to_string(),
        "spawned.0",
        &[config],
    );
    let launched = rig.launched();
    assert!(
        launched.contains(&expected),
        "spawn must hand over scoped context: {launched}"
    );
    assert!(launched.contains("Spawn instructions"));
    assert_policy_absent(&launched, POLICY);
}

#[test]
fn scoped_policy_a_failed_reference_write_refuses_before_starting_a_worker() {
    let rig = super::seat_relaunch::Rig::new("hpol-write");
    let (config, original) = bootstrap_live_reference(&rig);
    std::fs::write(&config, format!("{original}{}", config_with_policy(POLICY)))
        .expect("policy migrated after initial launch");
    let workspace = rig.dir.join("workspace.md");
    std::fs::remove_file(&workspace).expect("replace fixture target with an unwritable shape");
    std::fs::create_dir(&workspace).expect("directory cannot be replaced as a regular file");
    let (code, out, err) = rig.run(
        ae::cli::SPAWN,
        &[
            "scout",
            "--using",
            "fake-claude",
            "--",
            "bounded fixture task",
        ],
    );
    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        rig.launched().is_empty(),
        "do not start a worker with a dangling pointer"
    );
}

#[test]
fn scoped_policy_failed_relaunch_names_reference_and_never_starts_successor() {
    let rig = super::seat_relaunch::Rig::new("hpol-rfail");
    let (config, original) = bootstrap_live_reference(&rig);
    let pane = rig.seat("spawned.0", "scout", "claude");
    rig.kill_tools(&pane);
    let before = rig.launched();
    let workspace = rig.dir.join("workspace.md");
    std::fs::remove_file(&workspace).expect("fixture target removed");
    std::fs::create_dir(&workspace).expect("unwritable target shape");
    std::fs::write(&config, format!("{original}{}", config_with_policy(POLICY)))
        .expect("policy added after worker stopped");
    let (code, out, err) = rig.run("_relaunch", &["scout"]);
    assert_eq!(code, Some(1), "out={out} err={err}");
    assert_eq!(
        rig.launched(),
        before,
        "refusal must precede successor exec"
    );
    let (read, frame) = rig.tmux(&["capture-pane", "-p", "-t", &pane]);
    assert!(read, "refused worker pane must remain readable");
    assert!(
        format!("{out}{err}{frame}").contains("workspace.md"),
        "refusal must name target in caller or pane: out={out} err={err} pane={frame}"
    );
}

#[test]
fn scoped_policy_no_key_keeps_run_read_only_and_spawn_ignores_reference_write_failure() {
    let rig = super::seat_relaunch::Rig::new("hpol-absent");
    let workspace = rig.dir.join("workspace.md");
    let sentinel = "EXISTING-NO-KEY-REFERENCE-7d4885\n";
    std::fs::write(&workspace, sentinel).expect("existing reference sentinel");
    let pane = rig.seat("spawned.0", "old", "claude");
    assert_eq!(
        std::fs::read_to_string(&workspace).expect("reference after actual _run"),
        sentinel,
        "_run must introduce no manifest write when the key is absent"
    );
    rig.kill_tools(&pane);
    std::fs::remove_file(&workspace).expect("fixture target removed");
    std::fs::create_dir(&workspace).expect("unwritable target shape");
    let (code, out, err) = rig.run(
        ae::cli::SPAWN,
        &[
            "scout",
            "--using",
            "fake-claude",
            "--",
            "bounded fixture task",
        ],
    );
    assert_eq!(
        code,
        Some(0),
        "no-key behavior stays unchanged: out={out} err={err}"
    );
    assert!(
        !rig.launched().contains("Spawn instructions"),
        "no key means no pointer, so no new target-write requirement"
    );
}
