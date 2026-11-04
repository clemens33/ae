//! Frozen locked live-path pins at 7004fa85. Oracle: commands.md live/stopped
//! rename contract and events.md publication-before-notice order. Exact lines,
//! refusal precedence and partial-failure retries are labelled base behaviour.

use super::*;

fn live(tag: &str) -> Rig {
    let rig = Rig::new(tag);
    ae::meta::rewrite(&rig.dir, "watchdog", Some("false")).expect("watchdog off");
    write_exec(&rig.dir.join("events-tail"), RENAME_MONITOR_CORE);
    rig
}

fn id(rig: &Rig, name: &str) -> String {
    let (ok, text) = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        &format!("={name}:"),
        "#{session_id}",
    ]);
    assert!(ok, "session identity: {text}");
    text.trim().to_owned()
}

fn no_intent(rig: &Rig, new: &str) {
    assert!(
        !rig.home
            .join("sessions")
            .join(format!(".rename.{}.{new}.intent", rig.name))
            .exists(),
        "live rename writes no stopped carrier"
    );
}

fn rename_events(dir: &Path) -> Vec<ae::events::Event> {
    ae::store::open(dir)
        .container()
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| ae::events::Event::parse_line(line).ok())
        .filter(|event| event.action == "rename")
        .collect()
}

fn pending_request(rig: &Rig) {
    let pending = format!(
        r#"{{"ts":"2026-10-10T10:00:00Z","actor":"lead","actor_slot":"main","actor_session":"{}","action":"ask","target":"worker","target_slot":"worker.0","target_session":"{}","ref":"rl-pending","summary":"pending"}}"#,
        rig.name, rig.name
    );
    ae::store::open(&rig.dir)
        .append_event(&format!("{pending}\n"))
        .expect("pending request");
    assert_eq!(
        ae::session::SessionRead::open(&rig.dir)
            .expect("read pending fixture")
            .pending
            .len(),
        1,
        "complete legacy request is really pending"
    );
}

#[test]
fn locked_live_crash_table_refuses_all_six_stopped_cuts_before_any_mutation() {
    let rig = live("rlcuts");
    pending_request(&rig);
    let destination = rig.home.join("sessions/fresh");
    std::fs::create_dir(&destination).expect("destination collision");
    std::fs::write(destination.join("sentinel"), b"keep\n").expect("occupant");
    assert!(rig.tmux(&["new-session", "-d", "-s", "fresh", "sh"]).0);
    let before = super::guard_spec::tree(&rig.dir);
    let destination_before = super::guard_spec::tree(&destination);
    let original_id = id(&rig, &rig.name);
    let destination_id = id(&rig, "fresh");
    for boundary in [
        "after-intent",
        "after-work-move",
        "after-state-move",
        "after-meta",
        "after-assets",
        "after-result",
    ] {
        let result = rig.run_with_env(
            &[("AE_TEST_RENAME_CRASH_AT", boundary)],
            &["rename", &rig.name, "fresh"],
        );
        assert_eq!(
            result,
            (
                Some(1),
                String::new(),
                format!(
                    "Error: AE_TEST_RENAME_CRASH_AT='{boundary}' is armed for a stopped rename, but session '{}' is live — live crash cuts need a separate ruling. Nothing was renamed.\n",
                    rig.name
                )
            ),
            "live cut {boundary}"
        );
        assert_eq!(id(&rig, &rig.name), original_id);
        assert_eq!(
            super::guard_spec::tree(&rig.dir),
            before,
            "live cut leaves every state byte and mode untouched"
        );
        assert_eq!(super::guard_spec::tree(&destination), destination_before);
        assert_eq!(
            id(&rig, "fresh"),
            destination_id,
            "seam refusal precedes occupied destination and pending requests"
        );
        no_intent(&rig, "fresh");
    }
}

#[test]
fn locked_live_destination_precedence_is_tmux_then_path_then_pending_requests() {
    let rig = live("rlprecedence");
    pending_request(&rig);
    let before = super::guard_spec::tree(&rig.dir);
    let destination = rig.home.join("sessions/fresh");
    std::fs::create_dir(&destination).expect("occupied destination");
    std::fs::write(destination.join("sentinel"), b"keep\n").expect("occupant");
    assert!(rig.tmux(&["new-session", "-d", "-s", "fresh", "sh"]).0);
    let result = rig.run(&["rename", &rig.name, "fresh"]);
    assert_eq!(
        result,
        (
            Some(1),
            String::new(),
            "Error: session 'fresh' already exists.\n".into()
        ),
        "tmux collision beats path and requests"
    );
    assert!(rig.tmux(&["kill-session", "-t", "=fresh"]).0);
    let result = rig.run(&["rename", &rig.name, "fresh"]);
    assert_eq!(
        result,
        (
            Some(1),
            String::new(),
            format!(
                "Error: session directory '{}' already exists.\n",
                destination.display()
            )
        ),
        "path collision beats requests"
    );
    assert_eq!(
        std::fs::read(destination.join("sentinel")).expect("occupant"),
        b"keep\n"
    );
    std::fs::remove_dir_all(&destination).expect("remove fixture collision");
    let result = rig.run(&["rename", &rig.name, "fresh"]);
    assert_eq!(
        result,
        (
            Some(1),
            String::new(),
            format!(
                "Error: session '{}' has 1 pending request(s) a rename would strand (rl-pending ask in '{}') — close them first (reply from the target seat, or retire the holding seat), then retry. Nothing was renamed.\n",
                rig.name, rig.name
            )
        ),
        "pending request is the next refusal"
    );
    assert!(rig.session_is_live());
    assert_eq!(super::guard_spec::tree(&rig.dir), before);
    assert!(!destination.exists());
    no_intent(&rig, "fresh");
}

#[test]
fn locked_live_exact_id_moves_only_the_short_name_and_preserves_work() {
    let rig = live("rlprefix");
    // Local work at the root is displayed as the literal "/". Only meta is
    // changed here: the live rename must never move managed work.
    ae::meta::rewrite(&rig.dir, "work_dir", Some("/")).expect("known path fact");
    let server = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(rig.sock.clone()));
    assert!(
        ae::transport::observe_look(&server, &rig.name).is_some(),
        "readable look makes a redress observable"
    );
    assert!(
        rig.tmux(&[
            "set-option",
            "-t",
            &format!("={}:", rig.name),
            ae::theme::PATHS_OPTION,
            "stale-path-fact"
        ])
        .0
    );
    let longer = format!("{}ect", rig.name);
    assert!(rig.tmux(&["new-session", "-d", "-s", &longer, "sh"]).0);
    let short_id = id(&rig, &rig.name);
    let long_id = id(&rig, &longer);
    let before = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    std::fs::write(rig.home.join("work-sentinel"), b"caller work\n").expect("work sentinel");
    let result = rig.run(&["rename", &rig.name, "fresh"]);
    assert_eq!(
        result,
        (
            Some(0),
            format!(
                "Renamed '{}' → 'fresh'\nwatchdog off: 1 running seats NOT told — relaunch them or tell them.\n",
                rig.name
            ),
            String::new()
        )
    );
    assert_eq!(
        id(&rig, "fresh"),
        short_id,
        "only exact short identity moved"
    );
    assert_eq!(
        id(&rig, &longer),
        long_id,
        "prefix neighbour kept its identity"
    );
    let (ok, paths) = rig.tmux(&[
        "show-options",
        "-v",
        "-t",
        "=fresh:",
        ae::theme::PATHS_OPTION,
    ]);
    assert!(ok);
    assert_eq!(
        paths.trim(),
        "/",
        "successful rename republishes its path fact"
    );
    let new_dir = rig.home.join("sessions/fresh");
    let meta = std::fs::read_to_string(new_dir.join("meta")).expect("new meta");
    assert_eq!(
        meta,
        before.replace(&format!("session={}\n", rig.name), "session=fresh\n"),
        "live rename changes only the session row"
    );
    assert_eq!(
        std::fs::read(rig.home.join("work-sentinel")).expect("work"),
        b"caller work\n"
    );
    assert!(!rig.dir.exists());
    assert!(
        std::fs::read_to_string(new_dir.join("workspace.md"))
            .expect("published workspace")
            .contains("fresh")
    );
    let events = rename_events(&new_dir);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor, "ae:rename");
    assert_eq!(events[0].summary, Some(format!("{} -> fresh", rig.name)));
    no_intent(&rig, "fresh");
}

#[test]
fn locked_absent_short_name_routes_stopped_without_moving_its_live_prefix_neighbour() {
    let rig = live("rlstoppedprefix");
    let longer = format!("{}ect", rig.name);
    assert!(rig.tmux(&["new-session", "-d", "-s", &longer, "sh"]).0);
    let long_id = id(&rig, &longer);
    assert!(
        rig.tmux(&["kill-session", "-t", &format!("={}", rig.name)])
            .0
    );
    let result = rig.run(&["rename", &rig.name, "fresh"]);
    assert_eq!(
        result,
        (
            Some(0),
            format!(
                "Renamed '{}' → 'fresh' (stopped; work preserved at '{}').\n",
                rig.name,
                rig.home.display()
            ),
            String::new()
        )
    );
    assert_eq!(id(&rig, &longer), long_id);
    assert!(
        !rig.tmux(&["has-session", "-t", "=fresh"]).0,
        "stopped route starts no replacement session"
    );
    let carrier = std::fs::read_to_string(
        rig.home
            .join("sessions")
            .join(format!(".rename.{}.fresh.intent", rig.name)),
    )
    .expect("stopped result");
    assert!(carrier.lines().any(|line| line == "phase=complete"));
    assert!(
        rename_events(&rig.home.join("sessions/fresh")).is_empty(),
        "stopped rename writes no live notice record"
    );
}

#[test]
fn locked_live_without_meta_has_only_plain_success_and_no_publication() {
    let rig = AmbientRig::new("rlbare");
    let old = rig.plant("old", "watchdog=false\n");
    std::fs::remove_file(old.join("meta")).expect("no meta");
    std::fs::write(old.join("retained"), b"keep\n").expect("retained state");
    let before = super::guard_spec::tree(&old);
    let (_, original) = rig.tmux(&["display-message", "-p", "-t", "=old:", "#{session_id}"]);
    assert_eq!(
        rig.run(&["rename", "old", "new"]),
        (Some(0), "Renamed 'old' → 'new'\n".into(), String::new())
    );
    let (_, renamed) = rig.tmux(&["display-message", "-p", "-t", "=new:", "#{session_id}"]);
    assert_eq!(renamed, original);
    let new = rig.home.join("sessions/new");
    assert_eq!(
        super::guard_spec::tree(&new),
        before,
        "no meta means no assets, journal or helpers published"
    );
    assert!(!old.exists());
    assert!(!rig.home.join("sessions/.rename.old.new.intent").exists());
}

#[test]
fn locked_live_meta_failure_keeps_the_moved_state_and_retry_refuses_missing_old() {
    let rig = live("rlmetafail");
    std::fs::remove_file(ae::store::open(&rig.dir).meta_lock())
        .expect("remove released fixture lock");
    std::fs::create_dir(ae::store::open(&rig.dir).meta_lock()).expect("meta lock obstruction");
    let before = super::guard_spec::tree(&rig.dir);
    let original = id(&rig, &rig.name);
    let new_dir = rig.home.join("sessions/fresh");
    let reason = format!(
        "{}: the lock path is a directory — refused before any open",
        new_dir.join("meta.lock").display()
    );
    let result = rig.run(&["rename", &rig.name, "fresh"]);
    assert_eq!(
        result,
        (
            Some(1),
            String::new(),
            format!(
                "Error: 'fresh' was renamed but its meta still says '{}' ({reason}). Retry 'ae rename {} fresh' after fixing it.\n",
                rig.name, rig.name
            )
        )
    );
    assert_eq!(id(&rig, "fresh"), original);
    assert_eq!(
        super::guard_spec::tree(&new_dir),
        before,
        "meta failure comes before manifest and journal"
    );
    assert!(!rig.dir.exists());
    no_intent(&rig, "fresh");
    let retry = rig.run(&["rename", &rig.name, "fresh"]);
    assert_eq!(
        retry,
        (
            Some(1),
            String::new(),
            format!(
                "Error: session '{}' has no state directory. Nothing was renamed.\n",
                rig.name
            )
        )
    );
    assert_eq!(id(&rig, "fresh"), original);
    assert_eq!(super::guard_spec::tree(&new_dir), before);
}

#[test]
fn locked_live_journal_failure_still_publishes_and_reports_seats_not_told() {
    let rig = live("rljournalfail");
    let lock = ae::store::lock_path(&ae::store::open(&rig.dir).events_path());
    std::fs::create_dir(lock).expect("journal lock obstruction");
    let result = rig.run(&["rename", &rig.name, "fresh"]);
    let new_dir = rig.home.join("sessions/fresh");
    let lock = new_dir.join("events.jsonl.lock");
    assert_eq!(
        result,
        (
            Some(0),
            format!(
                "Renamed '{}' → 'fresh'\nrename not journaled: 1 running seats NOT told — relaunch them or tell them.\n",
                rig.name
            ),
            format!(
                "Warning: the rename record was not journaled (could not lock {} within 5s: {}: the lock path is a directory — refused before any open).\n",
                lock.display(),
                lock.display()
            )
        )
    );
    assert!(
        std::fs::read_to_string(new_dir.join("workspace.md"))
            .expect("manifest")
            .contains("fresh")
    );
    assert!(rename_events(&new_dir).is_empty());
    assert!(
        new_dir.join("events.jsonl.lock").is_dir(),
        "obstruction retained"
    );
    assert!(!rig.dir.exists());
    no_intent(&rig, "fresh");
}

#[test]
fn locked_live_manifest_failure_rebinds_events_before_refusing_and_skips_redress() {
    let rig = live("rlmanifestfail");
    let server = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(rig.sock.clone()));
    assert!(
        ae::transport::observe_look(&server, &rig.name).is_some(),
        "readable look makes a redress observable"
    );
    assert!(
        rig.tmux(&[
            "set-option",
            "-t",
            &format!("={}:", rig.name),
            ae::theme::PATHS_OPTION,
            "keep-this-path-fact"
        ])
        .0
    );
    std::fs::create_dir(rig.dir.join("workspace.md")).expect("manifest obstruction");
    let result = rig.run(&["rename", &rig.name, "fresh"]);
    assert_eq!(result.0, Some(1), "manifest refusal: {result:?}");
    assert!(result.1.is_empty(), "manifest failure cannot print success");
    assert!(result.2.starts_with("Error: 'fresh' was renamed but its workspace manifest could not be published (could not publish "), "manifest error is primary: {result:?}");
    assert!(
        result.2.ends_with(&format!(
            "; its running seats were NOT told. Retry 'ae rename {} fresh' after fixing it.\n",
            rig.name
        )),
        "manifest error suffix: {result:?}"
    );
    let new_dir = rig.home.join("sessions/fresh");
    let (ok, panes) = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        "=fresh",
        "-F",
        "#{@ae_agent}|#{pane_start_command}",
    ]);
    assert!(
        ok && panes.lines().any(|line| line.starts_with("_events|")
            && line.contains(&new_dir.join("events-tail").display().to_string())),
        "events rebound before manifest failure: {panes}"
    );
    let (ok, paths) = rig.tmux(&[
        "show-options",
        "-v",
        "-t",
        "=fresh:",
        ae::theme::PATHS_OPTION,
    ]);
    assert!(ok);
    assert_eq!(
        paths.trim(),
        "keep-this-path-fact",
        "manifest refusal skips look-fact republication"
    );
    assert!(
        rename_events(&new_dir).is_empty(),
        "unpublished manifest never arms notice"
    );
    assert!(new_dir.join("workspace.md").is_dir());
    assert!(!rig.dir.exists());
    no_intent(&rig, "fresh");
}
