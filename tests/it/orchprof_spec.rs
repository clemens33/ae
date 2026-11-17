//! W1 acceptance, authored independently before implementation.
//! The oracles are literal fixture bytes, the ruled file policy and actual
//! launch/resume metadata. No live config or model process is used.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns and inspects scratch files and a private tmux server"
)]

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
use std::path::Path;
use std::process::Stdio;

use ae::config::{apply_roster_row, read_config_seen, set_roster_row};

use super::{OwnedScratch, Rig, ae, skip, stop};

const INVENTORY: &str = "[profiles]\nold = /bin/sleep 600\nnew = /bin/sleep 601\n";
const BASE: &str = "[profiles]\nold = /bin/sleep 600\nnew = /bin/sleep 601\n[roster]\norchestrator = old\n[workspace]\nmain = orchestrator\nworkers = \"\"\n";

fn file(tag: &str, bytes: &[u8]) -> (OwnedScratch, std::path::PathBuf) {
    let scratch = OwnedScratch::root("op", tag);
    let path = scratch.join("config");
    fs::write(&path, bytes).expect("scratch config");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("private config");
    (scratch, path)
}

fn names(root: &Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = fs::read_dir(root)
        .expect("scratch entries")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    names.sort();
    names
}

#[test]
fn r1_only_the_selected_value_changes_across_lossless_layouts() {
    for (row, expected) in [
        ("orchestrator = old\n", "orchestrator = new\n"),
        (
            "  orchestrator\t =\told  # keep # old\n",
            "  orchestrator\t =\tnew  # keep # old\n",
        ),
        ("orchestrator = \"old\"  \n", "orchestrator = \"new\"  \n"),
        ("orchestrator=old", "orchestrator=new"),
        ("orchestrator = old\r\n", "orchestrator = new\r\n"),
    ] {
        let prefix = format!("# old stays\n{INVENTORY}[roster]\nhelper = old\n");
        let input = format!("{prefix}{row}");
        let edit = set_roster_row(&input, "orchestrator", "new").expect("placeable row");
        assert!(edit.changed, "{row:?}");
        assert_eq!(edit.text, format!("{prefix}{expected}"), "{row:?}");
    }
    let input = BASE.replace('\n', "\r\n");
    let edit = set_roster_row(&input, "orchestrator", "new").expect("all-CRLF config");
    assert_eq!(
        edit.text,
        input.replace("orchestrator = old", "orchestrator = new")
    );
}

#[test]
fn r2_prompt_blocks_are_opaque_and_the_real_row_is_selected() {
    for key in ["instructions", "spawn_instructions"] {
        let prefix = format!(
            "{INVENTORY}[prompt]\n{key} = \"\"\"\n[roster]\norchestrator = old\n# raw prose\n\"\"\"\n"
        );
        let input = format!("{prefix}[roster]\norchestrator = old # real\n");
        let edit = set_roster_row(&input, "orchestrator", "new").expect("real row after block");
        assert_eq!(
            edit.text,
            format!("{prefix}[roster]\norchestrator = new # real\n")
        );
        assert!(
            set_roster_row(&prefix, "orchestrator", "new").is_err(),
            "block-only row is absent"
        );
    }
}

#[test]
fn r3_ambiguous_missing_and_unplaceable_rows_refuse() {
    for suffix in [
        "[workspace]\norchestrator = true\n",
        "[roster]\n# orchestrator = old\n",
        "[roster]\norchestrator = old\norchestrator = old\n",
        "[roster]\norchestrator = old\n[roster]\norchestrator = old\n",
        "[roster]\norchestrator =\n",
        "[roster]\norchestrator = old\norchestrator =\n",
        "[roster]\norchestrator = \"old\" # unsupported quoted comment\n",
        "[roster]\norchestrator = \"\"\"\nold\n\"\"\"\n",
        "[prompt]\ninstructions = \"\"\"\n[roster]\norchestrator = old\n",
    ] {
        let input = format!("{INVENTORY}{suffix}");
        let refusal =
            set_roster_row(&input, "orchestrator", "new").expect_err("unsafe layout refuses");
        assert!(
            !refusal.to_string().trim().is_empty(),
            "human-readable refusal"
        );
    }
}

#[test]
fn r4_profiles_are_inventory_keys_and_never_config_injection() {
    for profile in [
        "missing",
        "",
        "new\n[workspace]",
        "new#tail",
        "\"new\"",
        "new other",
        "new@client",
    ] {
        assert!(
            set_roster_row(BASE, "orchestrator", profile).is_err(),
            "{profile:?}"
        );
    }
    let edit =
        set_roster_row(BASE, "orchestrator", "new").expect("defined profile without spend prose");
    assert!(edit.changed);
    let with_helper = format!("{BASE}\n[roster]\nhelper = old\n");
    assert!(
        set_roster_row(&with_helper, "helper", "new").is_err(),
        "only the orchestrator row is editable"
    );
}

#[test]
fn r5_the_real_config_launch_plan_resolves_the_new_binding_only() {
    let input = format!("{BASE}\n[prompt]\ninstructions = untouched\n[roster]\nhelper = old\n");
    let edit = set_roster_row(&input, "orchestrator", "new").expect("rewrite");
    let before = ae::config::parse_identity(&input).expect("before identity");
    let after = ae::config::parse_identity(&edit.text).expect("after identity");
    assert_eq!(before.clients, after.clients);
    assert_eq!(before.profiles, after.profiles);
    assert_eq!(before.main, after.main);
    assert_eq!(before.workers, after.workers);
    assert_eq!(before.effort_floor, after.effort_floor);
    assert_eq!(
        after.roster,
        vec![
            ("orchestrator".to_owned(), "new".to_owned()),
            ("helper".to_owned(), "old".to_owned())
        ]
    );
    let plan = ae::config::launch_plan(&after, Some("orchestrator"), None).expect("real resolver");
    assert_eq!(plan.seats[0].profile, "new");
    assert_eq!(plan.seats[0].command.as_str(), "/bin/sleep 601");
}

#[test]
fn r6_same_profile_is_byte_identical() {
    let input = BASE.replace("orchestrator = old", "orchestrator\t= \"old\"  ");
    let edit = set_roster_row(&input, "orchestrator", "old").expect("same pick");
    assert!(!edit.changed);
    assert_eq!(edit.text, input);
}

#[test]
fn a1_a_symlink_refuses_at_read_and_again_at_apply() {
    let (scratch, path) = file("symlink", BASE.as_bytes());
    let seen = read_config_seen(&path).expect("original regular file");
    let target = scratch.join("target");
    fs::rename(&path, &target).expect("preserve original as link target");
    symlink(&target, &path).expect("config link");
    assert!(read_config_seen(&path).is_err(), "read refuses link");
    assert!(
        apply_roster_row(&path, &seen, "orchestrator", "new", 42).is_err(),
        "apply rechecks link"
    );
    assert!(
        fs::symlink_metadata(&path)
            .expect("link metadata")
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&target).expect("target bytes"), BASE.as_bytes());
    assert_eq!(names(&scratch).len(), 2, "no backup or temp");
}

#[test]
fn a2_success_keeps_mode_and_owner_and_makes_one_exact_backup() {
    for mode in [0o600, 0o640] {
        let (scratch, path) = file("mode", BASE.as_bytes());
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("original mode");
        let original = fs::metadata(&path).expect("original metadata");
        let seen = read_config_seen(&path).expect("display snapshot");
        let applied = apply_roster_row(&path, &seen, "orchestrator", "new", 42).expect("publish");
        assert!(applied.changed);
        let backup = applied.backup.expect("successful change names its backup");
        assert_ne!(backup, path);
        assert_eq!(backup.parent(), Some(scratch.as_ref()));
        assert_eq!(fs::read(&backup).expect("backup"), BASE.as_bytes());
        let published = fs::metadata(&path).expect("published metadata");
        assert_eq!(published.mode() & 0o7777, original.mode() & 0o7777);
        assert_eq!(
            (published.uid(), published.gid()),
            (original.uid(), original.gid())
        );
        assert_eq!(
            fs::read_to_string(&path).expect("published bytes"),
            BASE.replace("orchestrator = old", "orchestrator = new")
        );
        assert_eq!(
            names(&scratch).len(),
            2,
            "config plus exactly one backup, no temp"
        );
        let identity =
            ae::config::read_identity(Some(&path), None).expect("published config reader");
        let plan = ae::config::launch_plan(&identity, Some("orchestrator"), None)
            .expect("published start resolver");
        assert_eq!(plan.seats[0].profile, "new");
    }
}

#[test]
fn a3_changed_since_display_refuses_even_when_only_a_comment_changed() {
    let (scratch, path) = file("changed", BASE.as_bytes());
    let seen = read_config_seen(&path).expect("display snapshot");
    let current = format!("# a human edit\n{BASE}");
    fs::write(&path, &current).expect("concurrent human edit");
    for profile in ["new", "old"] {
        let why = apply_roster_row(&path, &seen, "orchestrator", profile, 42)
            .expect_err("stale apply refuses, including a same-profile pick");
        assert!(!why.to_string().trim().is_empty());
    }
    assert_eq!(fs::read_to_string(&path).expect("current bytes"), current);
    assert_eq!(names(&scratch).len(), 1, "refusal creates nothing");
}

#[test]
fn a4_a_removed_or_unknown_profile_never_publishes() {
    let current = BASE.replace("new = /bin/sleep 601\n", "");
    let (scratch, path) = file("profile", current.as_bytes());
    let seen = read_config_seen(&path).expect("current display");
    assert!(apply_roster_row(&path, &seen, "orchestrator", "new", 42).is_err());
    assert_eq!(fs::read_to_string(&path).expect("unchanged bytes"), current);
    assert_eq!(names(&scratch).len(), 1);
}

#[test]
fn a5_backup_collision_refuses_without_touching_any_existing_file() {
    let (scratch, path) = file("collision", BASE.as_bytes());
    let seen = read_config_seen(&path).expect("display snapshot");
    let first = apply_roster_row(&path, &seen, "orchestrator", "new", 42).expect("first apply");
    let backup = first.backup.expect("first backup");
    let current = fs::read(&path).expect("first published bytes");
    let seen = read_config_seen(&path).expect("fresh display snapshot");
    assert!(
        apply_roster_row(&path, &seen, "orchestrator", "old", 42).is_err(),
        "same backup name refuses"
    );
    assert_eq!(fs::read(&path).expect("config"), current);
    assert_eq!(fs::read(&backup).expect("backup"), BASE.as_bytes());
    assert_eq!(names(&scratch).len(), 2, "no stray temp");
}

#[test]
fn a6_same_pick_has_no_backup_write_or_replacement() {
    let (scratch, path) = file("noop", BASE.as_bytes());
    let original = fs::metadata(&path).expect("before metadata");
    let seen = read_config_seen(&path).expect("display snapshot");
    let applied =
        apply_roster_row(&path, &seen, "orchestrator", "old", 42).expect("no-op succeeds");
    assert!(!applied.changed);
    assert!(applied.backup.is_none());
    let after = fs::metadata(&path).expect("after metadata");
    assert_eq!(
        (
            after.dev(),
            after.ino(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec()
        ),
        (
            original.dev(),
            original.ino(),
            original.mtime(),
            original.mtime_nsec(),
            original.ctime(),
            original.ctime_nsec()
        )
    );
    assert_eq!(fs::read(&path).expect("original bytes"), BASE.as_bytes());
    assert_eq!(names(&scratch).len(), 1);
}

#[test]
fn a7_invalid_utf8_and_non_regular_selected_paths_refuse() {
    let (scratch, path) = file("invalid", b"[roster]\norchestrator = \xff\n");
    assert!(read_config_seen(&path).is_err());
    assert!(read_config_seen(&scratch).is_err());
    assert_eq!(
        fs::read(&path).expect("invalid bytes preserved"),
        b"[roster]\norchestrator = \xff\n"
    );
    assert_eq!(names(&scratch).len(), 1);
}

#[test]
fn a8_oversize_input_refuses_without_writing() {
    // The selected config reader has a 1 MiB cap. This input exceeds it.
    let bytes = vec![b'#'; 1024 * 1024 + 1];
    let (scratch, path) = file("oversize", &bytes);
    assert!(read_config_seen(&path).is_err());
    assert_eq!(fs::read(&path).expect("oversize input preserved"), bytes);
    assert_eq!(names(&scratch).len(), 1);
}

fn launch_orchestrator(rig: &Rig) {
    let overlay = rig.home.join("orchestrator.config");
    let out = ae()
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &rig.config)
        .env("TMUX_TMPDIR", &rig.scratch)
        .args([
            ae::cli::LAUNCH,
            "--home",
            &rig.home.display().to_string(),
            "--cwd",
            &rig.project.display().to_string(),
            "--global",
            &rig.config.display().to_string(),
            "--local-config",
            &overlay.display().to_string(),
            "--server-kind",
            "socket",
            "--server",
            &rig.sock.display().to_string(),
            "--no-attach",
            "--no-autostart",
            "--",
            "--local",
            "orchestrator",
        ])
        .output()
        .expect("real private launch");
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn s5_a_real_fresh_start_uses_the_pick_and_pause_resume_keeps_its_saved_profile() {
    if skip() {
        return;
    }
    let rig = Rig::idle("orchprof-real");
    fs::create_dir_all(&rig.home).expect("private ae home");
    fs::write(&rig.config, BASE).expect("global inventory");
    fs::write(rig.home.join("orchestrator.config"), "[profiles]\nlocal = /bin/sleep 602\n[roster]\norchestrator = local\n[workspace]\nmain = orchestrator\nworkers = \"\"\nwatchdog = false\nchat = off\norchestrator = true\n").expect("legacy local identity must be ignored");
    let seen = read_config_seen(&rig.config).expect("display snapshot");
    apply_roster_row(&rig.config, &seen, "orchestrator", "new", 42)
        .expect("pick before fresh start");
    launch_orchestrator(&rig);
    let meta = rig.meta("orchestrator");
    assert!(meta.contains("profile.main=new\n"), "{meta}");
    let seen = read_config_seen(&rig.config).expect("display during running seat");
    apply_roster_row(&rig.config, &seen, "orchestrator", "old", 43).expect("next-fresh-start pick");
    assert_eq!(
        rig.meta("orchestrator"),
        meta,
        "save does not mutate the running seat"
    );
    stop(&rig, "orchestrator");
    launch_orchestrator(&rig);
    let resumed = rig.meta("orchestrator");
    assert!(
        resumed.contains("profile.main=new\n"),
        "Resume keeps recorded profile: {resumed}"
    );
    let identity = ae::config::read_identity(Some(&rig.config), None).expect("current global");
    assert_eq!(identity.roster_profile("orchestrator"), Some("old"));

    // Answer the real confirmation only for this fixture's private session.
    let mut end = ae()
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &rig.config)
        .env("TMUX_TMPDIR", &rig.scratch)
        .args(["end", "orchestrator", "--keep-history"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("private end command");
    end.stdin
        .take()
        .expect("end confirmation input")
        .write_all(b"y\n")
        .expect("fixture confirmation");
    let ended = end.wait_with_output().expect("end completion");
    assert!(ended.status.success(), "{ended:?}");
    assert!(
        !rig.dir("orchestrator").exists(),
        "end removes the prior session state"
    );
    launch_orchestrator(&rig);
    let fresh = rig.meta("orchestrator");
    assert!(
        fresh.contains("profile.main=old\n"),
        "fresh start after end uses latest pick: {fresh}"
    );
}
