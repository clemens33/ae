//! Released orchestrator overlays and relay context compatibility through the real CLI.

use super::{
    ATTEMPT, Rig, Upgrade, context_recording_tool, events, meta, read, recorded_context, rig,
    stamp_config, stamp_launch, words,
};
use std::fs;
use std::path::{Path, PathBuf};

const OLDEST: &str = include_str!("../../fixtures/orchestrator/v2026.8.2.config");
const SEPTEMBER: &str = include_str!("../../fixtures/orchestrator/v2026.9.61.config");
const RECENT: &str = include_str!("../../fixtures/orchestrator/s1a-previous.config");
const CURRENT: &str = include_str!("../../../contrib/aeorchestrator/orchestrator.config");
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn current_with_released_spacing() -> String {
    CURRENT.replacen("\nsweep = 300\n", "\nsweep = 120\n", 1)
}

fn backup(path: &Path, version: &str, suffix: &str) -> PathBuf {
    path.with_file_name(format!("orchestrator.config.pre-v{version}{suffix}"))
}

fn published(upgrade: &Upgrade<'_>, version: &str) -> String {
    let out = upgrade.publish(version);
    let said = words(&out);
    assert_eq!(out.0, Some(0), "offline upgrade: {said}");
    said
}

fn one_line(said: &str, matches: impl Fn(&str) -> bool) -> &str {
    let mut lines = said.lines().filter(|line| matches(line));
    let line = lines
        .next()
        .unwrap_or_else(|| panic!("missing notice: {said}"));
    assert!(lines.next().is_none(), "expected one notice: {said}");
    line
}

fn migration_notice<'a>(said: &'a str, path: &Path, saved: &Path) -> &'a str {
    one_line(said, |line| {
        let saved = saved.to_str().expect("backup path");
        line.starts_with("ae: ")
            && line.contains(saved)
            && line
                .replace(saved, "")
                .contains(path.to_str().expect("fixture path"))
    })
}

#[test]
fn released_whole_overlays_are_replaced_and_backed_up_even_without_a_seat() {
    let r = rig("ocwhole");
    let u = Upgrade::new(&r);
    let path = u.state().join("orchestrator.config");
    assert!(!u.state().join("sessions/orchestrator").exists());
    let retained = current_with_released_spacing();
    for (old, version, expected) in [
        (OLDEST, "2026.10.60", CURRENT),
        (SEPTEMBER, "2026.10.61", retained.as_str()),
        (RECENT, "2026.10.62", retained.as_str()),
    ] {
        assert_ne!(old, CURRENT, "the positive fixture must require migration");
        fs::write(&path, old).expect("released overlay");
        let said = published(&u, version);
        assert_eq!(
            read(&path),
            expected,
            "publish did not install the current prompt with the expected sweep for {version}: {said}"
        );
        let saved = backup(&path, version, "");
        assert_eq!(
            fs::read(&saved).expect("backup beside overlay"),
            old.as_bytes()
        );
        migration_notice(&said, &path, &saved);
    }

    let said = published(&u, "2026.10.63");
    assert_eq!(read(&path), retained);
    assert!(!backup(&path, "2026.10.63", "").exists());
    assert!(
        !said.contains("orchestrator.config"),
        "already current: {said}"
    );
}

#[test]
fn a_known_old_stanza_keeps_owner_bytes_and_an_existing_backup() {
    let r = rig("ocstanza");
    let u = Upgrade::new(&r);
    let path = u.state().join("orchestrator.config");
    let header = "[prompt]\n";
    let (prefix, old_stanza) = SEPTEMBER.split_once(header).expect("old prompt");
    let (_, new_stanza) = CURRENT.split_once(header).expect("current prompt");
    let prefix = prefix.replace("layout = vertical", "layout = horizontal # owner edit");
    let suffix = "\n[workspace]\n# owner's note after the historical prompt\n";
    let old = format!("{prefix}{header}{old_stanza}{suffix}");
    let wanted = format!("{prefix}{header}{new_stanza}{suffix}");
    assert!(old.contains("# owner edit"));
    fs::write(&path, &old).expect("overlay with owner preferences");
    let occupied = backup(&path, VERSION, "");
    fs::write(&occupied, "a previous backup\n").expect("occupied backup name");

    let said = published(&u, VERSION);
    assert_eq!(
        read(&path),
        wanted,
        "edited workspace lost or old prompt kept: {said}"
    );
    assert_eq!(read(&occupied), "a previous backup\n");
    let saved = backup(&path, VERSION, "-2");
    assert_eq!(fs::read(&saved).expect("next free backup"), old.as_bytes());
    migration_notice(&said, &path, &saved);
}

fn repair_line(said: &str) -> &str {
    one_line(said, |line| {
        line.contains("orchestrator.config")
            && line.contains("ae stop orchestrator")
            && line.contains("ae orchestrator")
    })
}

#[test]
fn edited_stale_overlay_is_preserved_and_upgrade_and_relay_share_the_repair() {
    let r = rig("ocedited");
    let u = Upgrade::new(&r);
    let edited = SEPTEMBER.replace("You are the ae orchestrator:", "Owner's local prompt:");
    assert_ne!(edited, SEPTEMBER);
    let path = u.state().join("orchestrator.config");
    fs::write(&path, &edited).expect("owner edited stale prompt");
    let said = published(&u, VERSION);
    assert_eq!(read(&path), edited);
    let upgrade_line = repair_line(&said);
    assert!(upgrade_line.starts_with("ae: "), "{upgrade_line}");
    let saved = backup(&path, VERSION, "");
    assert!(
        upgrade_line.contains(saved.to_str().expect("backup path")),
        "{upgrade_line}"
    );
    assert!(
        !saved.exists(),
        "the warning must leave the file and backups alone"
    );

    let relay_path = r.world.root.join("orchestrator.config");
    fs::write(&relay_path, &edited).expect("same edited overlay for dispatcher");
    let claims = || {
        events(&r.caller.dir)
            .iter()
            .filter(|event| event.action == ATTEMPT)
            .count()
    };
    let before = claims();
    let out = r.relay("Fixture owner words.");
    let refused = words(&out);
    assert_ne!(out.0, Some(0), "{refused}");
    assert!(out.1.is_empty(), "unexpected relay receipt: {refused}");
    assert_eq!(claims(), before, "the refusal must precede a claim");
    assert!(
        r.asks().is_empty(),
        "the refusal must not open a target request"
    );
    let relay_line = repair_line(&refused)
        .strip_suffix(". Nothing was sent.")
        .expect("standard relay refusal suffix");
    let normalized = relay_line.replace(
        r.world.root.to_str().expect("relay fixture root"),
        u.state().to_str().expect("upgrade fixture root"),
    );
    let matching: String = upgrade_line
        .chars()
        .rev()
        .zip(normalized.chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let saved = saved.to_str().expect("backup path");
    assert!(
        matching
            .replace(saved, "")
            .contains(path.to_str().expect("overlay path"))
            && matching.contains(saved)
            && matching.contains("ae stop orchestrator")
            && matching.contains("ae orchestrator"),
        "upgrade and relay must print the same complete repair: {upgrade_line}\n{normalized}",
    );
}

#[test]
fn hostile_orchestrator_overlay_is_neither_rewritten_nor_silently_ignored() {
    let r = rig("ocbytes");
    let u = Upgrade::new(&r);
    let path = u.state().join("orchestrator.config");
    let invalid = b"[prompt]\r\ninstructions = \0\xff";
    fs::write(&path, invalid).expect("hostile hand-edited bytes");
    let said = published(&u, VERSION);
    assert_eq!(fs::read(&path).expect("unaltered bytes"), invalid);
    assert!(!backup(&path, VERSION, "").exists());
    let _ = repair_line(&said);
}

#[test]
fn an_owner_edit_to_a_compatible_prompt_stays_silent_and_untouched() {
    let r = rig("occurrent");
    let u = Upgrade::new(&r);
    let dir = u.plant(
        "",
        "orchestrator",
        ae::migrate::CURRENT,
        "meta_agent=true\n",
        false,
    );
    let path = u.state().join("orchestrator.config");
    meta(&dir, &format!("local_config={}\n", path.display()));
    let (body, after) = CURRENT
        .rsplit_once("\n\"\"\"")
        .expect("current prompt block");
    let edited = format!("{body}\nAn owner note in the current prompt.\n\"\"\"{after}");
    fs::write(&path, &edited).expect("owner-edited compatible prompt");

    let said = published(&u, VERSION);
    assert_eq!(read(&path), edited);
    assert!(!backup(&path, VERSION, "").exists());
    assert!(
        !said.contains("orchestrator.config"),
        "compatible prompt: {said}"
    );
}

#[test]
fn named_instance_overlay_migrates_and_its_repair_names_the_instance() {
    let r = rig("ocnamed");
    let u = Upgrade::new(&r);
    let instance = "orchcfg-blue";
    let dir = u.plant(
        instance,
        "orchestrator",
        ae::migrate::CURRENT,
        "meta_agent=true\n",
        false,
    );
    let path = u
        .state()
        .join("instances")
        .join(instance)
        .join("orchestrator.config");
    meta(&dir, &format!("local_config={}\n", path.display()));
    assert!(!u.state().join("orchestrator.config").exists());
    fs::write(&path, SEPTEMBER).expect("old named-instance template");

    let said = published(&u, "2026.10.60");
    assert_eq!(
        read(&path),
        current_with_released_spacing(),
        "publish did not update the named overlay and retain its sweep: {said}"
    );
    let saved = backup(&path, "2026.10.60", "");
    assert_eq!(
        fs::read(&saved).expect("named-instance backup"),
        SEPTEMBER.as_bytes()
    );
    let line = migration_notice(&said, &path, &saved);
    assert!(
        line.contains(&format!("instance {instance}")),
        "migration lacks the instance: {line}"
    );
    assert!(
        !said.contains("ae stop orchestrator"),
        "default command would target another root: {said}"
    );

    let edited = format!("{SEPTEMBER}# an owner edit in this instance\n");
    fs::write(&path, &edited).expect("owner-edited named-instance template");
    let said = published(&u, VERSION);
    assert_eq!(read(&path), edited);
    assert!(!backup(&path, VERSION, "").exists());
    let line = one_line(&said, |line| {
        line.starts_with("ae: ") && line.contains(path.to_str().expect("named overlay path"))
    });
    assert!(
        line.contains(&format!("instance {instance}")),
        "warning lacks the instance: {line}"
    );
    assert!(
        line.contains(
            backup(&path, VERSION, "")
                .to_str()
                .expect("named backup path")
        ),
        "warning lacks the backup step: {line}"
    );
    assert!(
        !line.contains("ae stop orchestrator"),
        "default command would target another root: {line}"
    );
}

#[test]
fn upgrading_the_overlay_leaves_a_running_orchestrator_pane_intact() {
    let r = rig("ocrun");
    let u = Upgrade::new(&r);
    let dir = u.plant(
        "",
        "orchestrator",
        ae::migrate::CURRENT,
        "meta_agent=true\n",
        true,
    );
    let path = u.state().join("orchestrator.config");
    fs::write(&path, SEPTEMBER).expect("incident overlay");
    let pane = || {
        let (ok, text) = r.world.tmux(
            "list-panes -t",
            &["orchestrator", "-F", "#{session_id} #{pane_id} #{pane_pid}"],
        );
        assert!(
            ok && !text.trim().is_empty(),
            "live orchestrator pane: {text}"
        );
        text
    };
    let before = pane();
    let id = before.split_whitespace().nth(1).expect("pane id");
    meta(
        &dir,
        &format!(
            "main_pane={id}\nlocal_config={}\nlaunch_id.main=before-upgrade\n",
            path.display()
        ),
    );
    let previous_helper = fs::read_link(dir.join("send")).expect("old helper");
    let said = published(&u, VERSION);
    assert_eq!(
        pane(),
        before,
        "installed core must leave the original seat alive"
    );
    assert!(read(&dir.join("meta")).contains("launch_id.main=before-upgrade\n"));
    assert_ne!(
        fs::read_link(dir.join("send")).expect("new helper"),
        previous_helper
    );
    assert_eq!(read(&path), current_with_released_spacing());
    migration_notice(&said, &path, &backup(&path, VERSION, ""));
}

fn drop_context(path: &Path, slots: &[&str]) {
    let original = read(path);
    let stale: String = original
        .split_inclusive('\n')
        .filter(|line| {
            !slots
                .iter()
                .any(|slot| line.starts_with(&format!("context.{slot}=")))
        })
        .collect();
    assert_ne!(
        stale, original,
        "the live fixture should have its initial context stamps"
    );
    fs::write(path, stale).expect("model a pre-context launch");
}

#[test]
fn relay_names_stop_resume_for_claude_and_fresh_conversation_for_muse() {
    let r = rig("ocroutes");
    let _ = stamp_config(&r, ["claude", "muse"]);
    stamp_launch(&r, &["orchcfg-target", "--no-attach"]);
    let dir = r.world.root.join("sessions/orchcfg-target");
    drop_context(&dir.join("meta"), &["main", "worker.0"]);
    let before = events(&dir);
    for (seat, slot) in [("lead", "main"), ("colead", "worker.0")] {
        let target = format!("orchcfg-target:{seat}");
        let out = r.run(&[
            &target,
            "--quote",
            "Fixture owner words.",
            "--task",
            "Fixture task.",
        ]);
        let said = words(&out);
        assert_ne!(out.0, Some(0), "{said}");
        assert!(
            said.contains(&format!(
                "target seat {slot} carries no current context stamp"
            )),
            "{said}"
        );
        if seat == "lead" {
            assert!(
                said.contains("ae stop orchcfg-target") && said.contains("ae orchcfg-target"),
                "{said}"
            );
            assert!(
                !said.contains("relaunch the target seat with a fresh conversation"),
                "Claude resume keeps the conversation: {said}"
            );
        } else {
            assert!(said.contains("fresh conversation"), "{said}");
            assert!(
                !said.contains("ae stop orchcfg-target"),
                "Muse exact resume cannot restamp: {said}"
            );
        }
        assert_eq!(
            events(&dir),
            before,
            "the refusal must leave the target unchanged"
        );
    }
}

fn launch_pair(r: &Rig, u: &Upgrade<'_>, state: &Path, name: &str) -> PathBuf {
    let output = crate::cli::ae()
        .env("HOME", &u.home)
        .env("AE_HOME", state)
        .env("CONFIG_FILE", state.join("config"))
        .env("TMUX_TMPDIR", &r.world.root)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &r.world.sock)
        .env("AE_NO_AUTOSTART", "1")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .current_dir(&u.home)
        .args([name, "--no-attach"])
        .output()
        .expect("real launch on the private server");
    assert!(output.status.success(), "launch {name}: {output:?}");
    state.join("sessions").join(name)
}

fn configure_pair(r: &Rig, state: &Path) -> PathBuf {
    let claude = context_recording_tool(&r.world.root.join("notice-claude"), "claude");
    let muse = context_recording_tool(&r.world.root.join("notice-muse"), "muse");
    fs::write(
        state.join("config"),
        format!(
            "[profiles]\nprimary = \"{}\"\nsecondary = \"{}\"\n\n\
             [roster]\nlead = primary\ncolead = secondary\n\n\
             [workspace]\nmain = lead\nworkers = colead\nlayout = lead-pair\n\
             watchdog = false\nchat = off\nauto_upgrade = off\nauto_trust = off\nquota = off\n",
            claude.display(),
            muse.display()
        ),
    )
    .expect("isolated config for the lead-pair tools");
    claude.with_file_name("received-context")
}

#[test]
fn upgrade_lists_running_leads_without_current_context_and_their_remedies() {
    let r = rig("ocnotice");
    let u = Upgrade::new(&r);
    let root = u.state();
    configure_pair(&r, &root);
    let stale = launch_pair(&r, &u, &root, "stale-orchcfg");
    let _current = launch_pair(&r, &u, &root, "current-orchcfg");
    let _stopped = u.plant(
        "",
        "stopped-orchcfg",
        ae::migrate::CURRENT,
        "agent_bin.main=claude\n",
        false,
    );
    drop_context(&stale.join("meta"), &["main", "worker.0"]);

    let said = published(&u, VERSION);
    let line = one_line(&said, |line| {
        line.starts_with("ae: ") && line.contains("stale-orchcfg") && line.contains("context stamp")
    });
    for required in [
        "main",
        "worker.0",
        "ae stop stale-orchcfg",
        "ae stale-orchcfg",
        "fresh conversation",
    ] {
        assert!(line.contains(required), "missing {required:?}: {line}");
    }
    assert!(
        !line.contains("current-orchcfg") && !line.contains("stopped-orchcfg"),
        "{line}"
    );
    let stale_after = read(&stale.join("meta"));
    assert!(!stale_after.contains("context.main=") && !stale_after.contains("context.worker.0="));
}

#[test]
fn named_instance_context_notice_routes_resume_to_that_instance() {
    let r = rig("ocnamedcontext");
    let u = Upgrade::new(&r);
    let instance = "orchcfg-blue";
    let name = "named-stale-lead";
    let root = u.state().join("instances").join(instance);
    fs::create_dir_all(&root).expect("named instance state");
    let claude_context = configure_pair(&r, &root);
    let dir = launch_pair(&r, &u, &root, name);
    let _ = recorded_context(&claude_context);
    drop_context(&dir.join("meta"), &["main"]);

    let said = published(&u, VERSION);
    let line = one_line(&said, |line| {
        line.starts_with("ae: ") && line.contains("context stamp") && line.contains(name)
    });
    assert!(
        line.contains(&format!("instance {instance}")),
        "context notice lost the session's instance: {line}"
    );
    for required in ["main", "stop and resume", "instance's own ae command"] {
        assert!(line.contains(required), "missing {required:?}: {line}");
    }
    for wrong_root in [format!("ae stop {name}"), format!("ae {name}")] {
        assert!(
            !line.contains(&wrong_root),
            "command would target the default root: {line}"
        );
    }
}
