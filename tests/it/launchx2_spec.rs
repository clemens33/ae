//! Characterisation of launch seams at 44a13587; no desired behaviour changes.
use super::*;

fn require_tmux() {
    let scratch = OwnedScratch::root("lx2", "tmux-required");
    assert!(
        tmux_present(&scratch),
        "launchx2 requires runnable tmux; no behavioural pin may silently skip"
    );
}

fn command(rig: &Rig, session: &str, local: Option<&Path>) -> Runner {
    let local = local.map(|path| path.display().to_string());
    let flags = local
        .as_deref()
        .map_or_else(Vec::new, |path| vec!["--local-config", path]);
    rig.launch_command_with_server_options(
        "socket",
        &rig.sock.display().to_string(),
        &["--local", session],
        &flags,
    )
}

#[test]
fn launchx2_suppressed_chat_does_not_warn_about_an_unusable_setting() {
    require_tmux();
    // Off-diagonal cases distinguish both independent suppression sources.
    for (meta_agent, dedicated) in [(false, false), (true, false), (false, true)] {
        let rig = Rig::idle("lx2-chat");
        let config = if dedicated {
            "[profiles]\nidle = \"sleep 600\"\n[roster]\norchestrator = idle\n[workspace]\nmain = orchestrator\nwatchdog = false\n".to_owned()
        } else {
            format!("{IDLE_CONFIG}chat = broken\nmeta = {meta_agent}\n")
        };
        std::fs::write(&rig.config, config).expect("private global identity");
        let overlay = rig.home.join("orchestrator.config");
        if dedicated {
            std::fs::create_dir_all(&rig.home).expect("dedicated home");
            std::fs::write(&overlay, "[workspace]\nchat = broken\n")
                .expect("dedicated unusable chat setting");
        }
        let session = if dedicated { "orchestrator" } else { "lx2Chat" };
        let result = command(&rig, session, dedicated.then_some(overlay.as_path()))
            .output()
            .expect("launch with unusable chat setting");
        assert!(result.status.success(), "{result:?}");
        let suppressed = meta_agent || dedicated;
        assert_eq!(
            String::from_utf8_lossy(&result.stderr),
            if suppressed {
                ""
            } else {
                "Note: [workspace] chat: \"broken\" is not on, off or app; the chat window opens, as when it is unset.\n"
            },
            "meta={meta_agent}, dedicated={dedicated}"
        );
        assert_eq!(
            rig.windows(session)
                .iter()
                .any(|(_, name, _)| name == "chat"),
            !suppressed,
            "meta={meta_agent}, dedicated={dedicated}"
        );
        assert_eq!(
            rig.meta(session)
                .lines()
                .any(|line| line == "meta_agent=true"),
            meta_agent,
            "dedicated suppression is independent of the meta-agent flag"
        );
    }
}

#[test]
fn launchx2_missing_and_empty_saved_layouts_fall_back_to_current_config() {
    require_tmux();
    for (saved, configured, expected) in [
        (None, "", "vertical"),
        (None, "horizontal", "horizontal"),
        (Some(""), "horizontal", "horizontal"),
        (Some("vertical"), "horizontal", "vertical"),
    ] {
        let rig = Rig::idle("lx2-layout");
        let session = "lx2Layout";
        std::fs::write(
            &rig.config,
            format!("{}chat = off\n", lead_config("vertical", &["worker"])),
        )
        .expect("initial two-seat layout");
        let created = rig.launch(&["--local", session]);
        assert_eq!(created.0, Some(0), "{created:?}");
        let _keeper = bare_session(&rig, &rig.sock, "keeper");
        stop(&rig, session);
        let mut meta = rig
            .meta(session)
            .lines()
            .filter(|line| !line.starts_with("layout="))
            .fold(String::new(), |mut text, line| {
                text.push_str(line);
                text.push('\n');
                text
            });
        if let Some(saved) = saved {
            writeln!(meta, "layout={saved}").expect("saved layout row");
        }
        std::fs::write(rig.dir(session).join("meta"), meta).expect("legacy layout record");
        std::fs::write(
            &rig.config,
            format!("{}chat = off\n", lead_config(configured, &["worker"])),
        )
        .expect("new layout setting");
        let resumed = rig.launch(&["--local", session]);
        assert_eq!(resumed.0, Some(0), "{resumed:?}");
        assert!(
            rig.meta(session)
                .lines()
                .any(|line| line == format!("layout={expected}")),
            "saved={saved:?}, configured={configured:?}: {}",
            rig.meta(session)
        );
        let (ok, geometry) = rig.tmux(&[
            "list-panes",
            "-t",
            &format!("={session}:0"),
            "-F",
            "#{pane_left}|#{pane_top}",
        ]);
        assert!(ok, "actual resumed pane geometry: {geometry}");
        let panes = geometry
            .lines()
            .map(|line| line.split_once('|').expect("left and top coordinates"))
            .collect::<Vec<_>>();
        assert_eq!(panes.len(), 2, "{geometry}");
        if expected == "vertical" {
            assert_ne!(panes[0].0, panes[1].0, "{geometry}");
            assert_eq!(panes[0].1, panes[1].1, "{geometry}");
        } else {
            assert_eq!(panes[0].0, panes[1].0, "{geometry}");
            assert_ne!(panes[0].1, panes[1].1, "{geometry}");
        }
    }
}

#[test]
fn launchx2_running_reattach_ignores_launch_only_config_errors_without_republishing() {
    require_tmux();
    let rig = Rig::idle("lx2-reattach");
    rig.without_chat();
    let session = "lx2Running";
    let base = std::fs::read_to_string(&rig.config).expect("initial config");
    let created = rig.launch(&["--local", session]);
    assert_eq!(created.0, Some(0), "{created:?}");
    rig.wait_for_pane_command(session, "main", "sleep");
    let meta = std::fs::read(rig.dir(session).join("meta")).expect("settled metadata");
    let events = std::fs::read(rig.dir(session).join("events.jsonl")).expect("launch journal");
    let panes = rig.panes(session);
    for (config, expected_fresh_code) in [
        (base.replace("lead = idle", "lead = missing"), 1),
        (format!("{base}quota_every_secs = broken\n"), 2),
        (base.replace("main = lead", "main = missing"), 1),
    ] {
        std::fs::write(&rig.config, config).expect("invalid launch-only config");
        let mut reattach = command(&rig, session, None);
        let log = launchx_trace_command(&mut reattach, &rig.scratch);
        let result = reattach.output().expect("running reattach");
        assert!(result.status.success(), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert!(
            String::from_utf8_lossy(&result.stdout)
                .starts_with("Session 'lx2Running' is running. Attach with: "),
            "{result:?}"
        );
        assert!(!launchx_trace_text(&log, &rig.scratch, false).contains("\tnew-session\t"));
        assert_eq!(rig.panes(session), panes, "reattach must not replace panes");
        assert_eq!(
            std::fs::read(rig.dir(session).join("meta")).expect("meta after reattach"),
            meta
        );
        assert_eq!(
            std::fs::read(rig.dir(session).join("events.jsonl")).expect("journal after reattach"),
            events
        );
        // A fresh launch sees the same broken setting, proving each fixture is active.
        let fresh = rig.launch(&["--local", "lx2FreshInvalid"]);
        assert_eq!(fresh.0, Some(expected_fresh_code), "{fresh:?}");
        assert!(!rig.dir("lx2FreshInvalid").exists());
    }
}

#[test]
fn launchx2_foreign_live_name_refuses_before_config_seed_or_session_state() {
    require_tmux();
    let rig = Rig::idle("lx2-foreign");
    let session = "lx2Foreign";
    let pane = bare_session(&rig, &rig.sock, session);
    std::fs::remove_file(&rig.config).expect("unseeded config");
    let mut launch = command(&rig, session, None);
    let log = launchx_trace_command(&mut launch, &rig.scratch);
    let result = launch.output().expect("foreign-session refusal");
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(result.stdout.is_empty(), "{result:?}");
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "Error: tmux session 'lx2Foreign' exists but is not an ae session.\n"
    );
    assert!(!rig.config.exists(), "refusal seeded config");
    assert!(
        !rig.dir(session).exists(),
        "refusal published session state"
    );
    assert!(
        !rig.home
            .join("sessions/.lifecycle.lx2Foreign.lock")
            .exists()
    );
    let trace = launchx_trace_text(&log, &rig.scratch, false);
    assert!(
        !trace.contains("\tnew-session\t") && !trace.contains("\tkill-session\t"),
        "{trace}"
    );
    let (ok, still_there) = rig.tmux(&["display-message", "-p", "-t", &pane, "#{pane_id}"]);
    assert!(
        ok && still_there.trim() == pane,
        "foreign pane was touched: {still_there}"
    );
}

#[test]
fn launchx2_reserved_main_and_worker_actors_refuse_before_default_seed() {
    require_tmux();
    for (actor, worker) in [("human", false), ("watchdog", true), ("ae", false)] {
        let rig = Rig::idle("lx2-actor");
        let session = "lx2Actor";
        std::fs::remove_file(&rig.config).expect("unseeded global config");
        let local = rig.scratch.join("identity-overlay");
        std::fs::write(&local, format!(
            "[profiles]\nidle = \"sleep 600\"\n[roster]\nlead = idle\n{actor} = idle\n[workspace]\nmain = {}\nworkers = {}\n",
            if worker { "lead" } else { actor }, if worker { actor } else { "" },
        )).expect("reserved identity local overlay");
        let mut launch = command(&rig, session, Some(&local));
        let log = launchx_trace_command(&mut launch, &rig.scratch);
        let result = launch.output().expect("reserved-actor refusal");
        assert_eq!(result.status.code(), Some(2), "{result:?}");
        assert!(result.stdout.is_empty(), "{result:?}");
        assert_eq!(
            String::from_utf8_lossy(&result.stderr),
            format!(
                "Error: agent name '{actor}' is reserved for ae's journal actors; pick another name.\n"
            )
        );
        assert!(
            !rig.config.exists(),
            "reserved actor seeded the global config"
        );
        assert!(!rig.dir(session).exists(), "reserved actor published state");
        assert!(!rig.home.join("sessions/.lifecycle.lx2Actor.lock").exists());
        assert!(!launchx_trace_text(&log, &rig.scratch, false).contains("\tnew-session\t"));
    }
}

#[test]
fn launchx2_resume_resolves_profiles_from_recorded_config_not_callers_config_file() {
    require_tmux();
    let rig = Rig::idle("lx2-config");
    rig.without_chat();
    let caller_config = rig.scratch.join("caller.config");
    std::fs::write(
        &caller_config,
        "[profiles]\nidle = \"cat\"\ncaller = \"cat\"\n\n[roster]\nlead = caller\n\n\
         [workspace]\nmain = lead\nchat = off\nwatchdog = false\n",
    )
    .expect("different caller roster and profile command");
    let launch = |session: &str, config: &Path| {
        ae().env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", &rig.scratch)
            .env("AE_HOME", &rig.home)
            .env("CONFIG_FILE", config)
            .env("AE_NO_AUTOSTART", "1")
            .env("TMUX_TMPDIR", &rig.scratch)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &rig.sock)
            .current_dir(&rig.project)
            .args(["--local", session, "--no-attach"])
            .output()
            .expect("public launch with an explicit CONFIG_FILE")
    };
    let session = "lx2RecordedConfig";
    let created = launch(session, &rig.config);
    assert!(created.status.success(), "{created:?}");
    rig.wait_for_pane_command(session, "main", "sleep");
    let config_row = format!("config={}", rig.config.display());
    assert!(rig.meta(session).lines().any(|line| line == config_row));

    // The caller config is valid and selects a different executable on a fresh
    // launch. Keeping that session alive also preserves the private server.
    let fresh = launch("lx2CallerConfig", &caller_config);
    assert!(fresh.status.success(), "{fresh:?}");
    rig.wait_for_pane_command("lx2CallerConfig", "main", "cat");
    assert!(
        rig.meta("lx2CallerConfig")
            .lines()
            .any(|line| line == "profile.main=caller")
    );
    stop(&rig, session);

    let resumed = launch(session, &caller_config);
    assert!(resumed.status.success(), "{resumed:?}");
    assert!(
        String::from_utf8_lossy(&resumed.stdout).contains("Resuming session lx2RecordedConfig"),
        "{resumed:?}"
    );
    let meta = rig.meta(session);
    assert!(
        meta.lines().any(|line| line == config_row),
        "resume must retain the recorded global config: {meta}"
    );
    assert!(
        meta.lines().any(|line| line == "profile.main=idle"),
        "{meta}"
    );
    rig.wait_for_pane_command(session, "main", "sleep");
}

#[test]
fn launchx2_orchestrator_notes_each_ignored_identity_section_independently() {
    require_tmux();
    for (section, overlay) in [
        ("profiles", "[profiles]\nidle = \"cat\"\n"),
        ("roster", "[roster]\norchestrator = missing\n"),
    ] {
        let rig = Rig::idle("lx2-note");
        std::fs::write(
            &rig.config,
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\norchestrator = idle\n\n\
             [workspace]\nmain = orchestrator\nchat = off\nwatchdog = false\n",
        )
        .expect("global orchestrator identity");
        std::fs::create_dir_all(&rig.home).expect("dedicated home");
        let local = rig.home.join("orchestrator.config");
        std::fs::write(&local, overlay).expect("one legacy identity section only");
        let result = command(&rig, "orchestrator", Some(&local))
            .output()
            .expect("launch with one ignored identity section");
        assert!(result.status.success(), "section={section}: {result:?}");
        assert_eq!(
            String::from_utf8_lossy(&result.stderr),
            format!(
                "ae orchestrator: ignoring [roster]/[profiles] in {}: the seat profile is [roster] orchestrator in {}\n",
                local.display(),
                rig.config.display()
            ),
            "the {section}-only overlay must independently trigger the note"
        );
        assert!(
            rig.meta("orchestrator")
                .lines()
                .any(|line| line == "profile.main=idle"),
            "section={section}: global identity remains authoritative"
        );
        rig.wait_for_pane_command("orchestrator", "main", "sleep");
        assert_eq!(
            std::fs::read_to_string(&local).expect("legacy overlay after launch"),
            overlay,
            "section={section}: note leaves the legacy overlay untouched"
        );
    }
}
