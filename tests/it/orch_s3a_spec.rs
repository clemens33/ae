//! Amendment A2 and the lead's state-guard ruling. Public argv, private tmux,
//! and on-disk state. The target UUID comes from the fleet JSON the seat uses.

#![allow(clippy::expect_used, reason = "fixture setup and effect checks")]

use super::super::o2guard_p2_spec::{Seat, World};
use super::*;
use ae::json::{self, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::mem::ManuallyDrop;

const STALE: &str = "11111111-2222-4333-8444-555555555555";
const TARGET: &str = "bound-target";
const REASON: &str = "The private test fixture records this state as the seat's latest declaration in the actual session journal.";
type Out = (Option<i32>, String, String);

fn rig(tag: &str) -> Rig {
    let rig = Rig::idle(tag);
    let pair = IDLE_CONFIG
        .replace("lead = idle\n", "lead = idle\ncolead = idle\n")
        .replace("layout = vertical\n", "layout = lead-pair\n");
    fs::write(
        &rig.config,
        format!("{pair}workers = colead\nchat = off\nauto_upgrade = off\nquota = off\n"),
    )
    .expect("inert lead-pair config");
    rig
}

fn dispatcher(rig: &Rig) -> Seat {
    let world = ManuallyDrop::new(World {
        root: rig.home.clone(),
        sock: rig.sock.clone(),
    });
    let overlay = rig.home.join("orchestrator.config");
    fs::create_dir_all(&rig.home).expect("private ae home");
    fs::write(
        &overlay,
        include_str!("../../contrib/aeorchestrator/orchestrator.config"),
    )
    .expect("real dispatcher overlay");
    let seat = world.session(
        "orch",
        "boss",
        &format!("meta_agent=true\nlocal_config={}\n", overlay.display()),
    );
    let path = seat.dir.join("meta");
    let meta = fs::read_to_string(&path).expect("dispatcher record");
    fs::write(path, format!("{meta}main_pane={}\n", seat.pane))
        .expect("record the dispatcher pane");
    seat
}

fn command(rig: &Rig, pane: Option<&str>, argv: &[&str]) -> Runner {
    let mut cmd = ae();
    cmd.env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &rig.config)
        .env("TMUX_TMPDIR", &rig.scratch)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &rig.sock)
        .env("AE_NO_AUTOSTART", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .current_dir(&rig.project)
        .args(argv);
    if let Some(pane) = pane {
        cmd.env("TMUX", format!("{},0,0", rig.sock.display()))
            .env("TMUX_PANE", pane);
    }
    cmd
}

fn output(cmd: &mut Runner) -> Out {
    let out = cmd.output().expect("run the real private ae binary");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn run(rig: &Rig, pane: Option<&str>, argv: &[&str]) -> Out {
    output(&mut command(rig, pane, argv))
}

fn succeeds(out: &Out) {
    assert_eq!(out.0, Some(0), "{out:?}");
}

fn census_uuid(rig: &Rig, name: &str) -> String {
    let out = run(rig, None, &["list", "--all", "--json"]);
    succeeds(&out);
    let list = json::parse(&out.1).expect("fleet JSON");
    let Some(Value::Arr(rows)) = list.get("sessions") else {
        panic!("missing session rows: {out:?}");
    };
    rows.iter()
        .find(|row| row.get_str("name") == Some(name))
        .and_then(|row| row.get_str("uuid"))
        .filter(|uuid| uuid.len() == 36)
        .expect("recorded full session UUID")
        .to_owned()
}

fn create(rig: &Rig, name: &str) -> String {
    succeeds(&run(rig, None, &[name, "--local", "--no-attach"]));
    rig.wait_for_pane_command(name, "main", "sleep");
    census_uuid(rig, name)
}

fn live(rig: &Rig, name: &str) -> bool {
    rig.sessions().iter().any(|row| row == name)
}

#[derive(Debug, PartialEq, Eq)]
enum Node {
    Directory,
    File(Vec<u8>),
    Link(PathBuf),
}

fn tree(root: &Path, path: &Path, nodes: &mut BTreeMap<PathBuf, Node>) {
    for entry in fs::read_dir(path).expect("private session tree") {
        let entry = entry.expect("private session entry");
        let full = entry.path();
        let rel = full.strip_prefix(root).expect("root relative");
        let kind = entry.file_type().expect("entry type");
        let node = if kind.is_dir() {
            tree(root, &full, nodes);
            Node::Directory
        } else if kind.is_symlink() {
            Node::Link(fs::read_link(&full).expect("link address"))
        } else {
            let bytes = fs::read(&full).expect("file bytes");
            // Taking a lifecycle lock can leave its empty root-level file.
            if bytes.is_empty()
                && path == root
                && rel
                    .to_str()
                    .is_some_and(|name| name.starts_with(".lifecycle."))
                && rel.extension().is_some_and(|ext| ext == "lock")
            {
                continue;
            }
            Node::File(bytes)
        };
        nodes.insert(rel.to_path_buf(), node);
    }
}

fn snapshot(rig: &Rig) -> (BTreeMap<PathBuf, Node>, String) {
    let root = rig.home.join("sessions");
    let mut nodes = BTreeMap::new();
    tree(&root, &root, &mut nodes);
    let (ok, sessions) = rig.tmux(&[
        "list-sessions",
        "-F",
        "#{session_name}|#{session_id}|#{@ae_session_uuid}",
    ]);
    assert!(ok, "private server must still answer: {sessions}");
    (nodes, sessions)
}

fn refuses(rig: &Rig, pane: Option<&str>, argv: &[&str]) -> Out {
    let before = snapshot(rig);
    let out = run(rig, pane, argv);
    assert!(matches!(out.0, Some(1 | 2)), "{argv:?}: {out:?}");
    assert_eq!(out.1, "", "{argv:?}: refusal wrote stdout");
    assert_eq!(snapshot(rig), before, "{argv:?}: refusal changed state");
    out
}

fn stop<'a>(name: &'a str, uuid: &'a str) -> [&'a str; 4] {
    ["stop", name, "--expect-uuid", uuid]
}

fn resume<'a>(name: &'a str, uuid: &'a str) -> [&'a str; 5] {
    [name, "--resume", "--no-attach", "--expect-uuid", uuid]
}

#[test]
fn dispatcher_can_stop_and_resume_the_uuid_from_the_fleet() {
    let rig = rig("s3round");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    let dir = rig.dir(TARGET);
    fs::write(dir.join("s3-sentinel"), b"keep this state").expect("target-local state");

    let out = run(&rig, Some(&caller.pane), &stop(TARGET, &uuid));
    succeeds(&out);
    assert!(!live(&rig, TARGET));
    assert!(live(&rig, "orch"));
    let events = fs::read_to_string(dir.join("events.jsonl")).expect("stop journal");
    assert!(events.contains("\"action\":\"stop-request\""), "{events}");
    assert!(events.contains("\"action\":\"stop-result\""), "{events}");
    assert_eq!(census_uuid(&rig, TARGET), uuid);

    succeeds(&run(&rig, Some(&caller.pane), &resume(TARGET, &uuid)));
    rig.wait_for_pane_command(TARGET, "main", "sleep");
    assert!(live(&rig, TARGET));
    assert_eq!(census_uuid(&rig, TARGET), uuid);
    assert_eq!(
        fs::read(dir.join("s3-sentinel")).expect("saved state"),
        b"keep this state"
    );
}

#[test]
fn stop_mismatch_names_the_actual_uuid_on_one_line_and_preserves_the_target() {
    let rig = rig("s3miss");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    assert_ne!(uuid, STALE);
    let out = refuses(&rig, Some(&caller.pane), &stop(TARGET, STALE));
    assert_eq!(out.2.lines().count(), 1, "{out:?}");
    assert!(out.2.contains(&uuid), "missing actual identity: {out:?}");
}

#[test]
fn bound_resume_requires_a_stopped_session_and_does_not_create() {
    let rig = rig("s3state");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    let out = refuses(&rig, Some(&caller.pane), &resume(TARGET, &uuid));
    assert!(out.2.contains("running"), "{out:?}");
    succeeds(&run(&rig, None, &["stop", TARGET]));
    let out = refuses(&rig, Some(&caller.pane), &resume(TARGET, STALE));
    assert!(out.2.contains(&uuid), "missing actual identity: {out:?}");
    refuses(&rig, Some(&caller.pane), &resume("never-recorded", &uuid));
    assert!(!rig.dir("never-recorded").exists());
}

#[test]
fn bound_stop_reports_a_proven_already_stopped_session() {
    let rig = rig("s3absent");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    succeeds(&run(&rig, None, &["stop", TARGET]));
    let path = rig.dir(TARGET).join("events.jsonl");
    let before = fs::read_to_string(&path).expect("journal before bound stop");
    let out = run(&rig, Some(&caller.pane), &stop(TARGET, &uuid));
    assert_eq!(
        out,
        (
            Some(1),
            String::new(),
            format!("Session '{TARGET}' is not running.\n")
        )
    );
    assert!(!live(&rig, TARGET));
    let after = fs::read_to_string(path).expect("journal after bound stop");
    let added = after.strip_prefix(&before).expect("append-only journal");
    assert_eq!(added.lines().count(), 1, "{added}");
    assert!(added.contains("\"action\":\"stop-result\""), "{added}");
    assert!(added.contains("already stopped"), "{added}");
}

#[test]
fn a_stale_name_cannot_follow_or_reclaim_a_live_rename_alias() {
    let rig = rig("s3alias");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    let new = "renamed-target";
    succeeds(&run(&rig, None, &["rename", TARGET, new]));
    assert!(
        fs::symlink_metadata(rig.dir(TARGET))
            .expect("old helper alias")
            .is_symlink()
    );
    assert_eq!(census_uuid(&rig, new), uuid);
    let pane = Some(caller.pane.as_str());
    refuses(&rig, pane, &stop(TARGET, &uuid));
    assert!(live(&rig, new));
    succeeds(&run(&rig, None, &["stop", new]));
    refuses(&rig, pane, &resume(TARGET, &uuid));
    assert!(!live(&rig, new));
    succeeds(&run(&rig, pane, &resume(new, &uuid)));
    assert!(live(&rig, new));
}

#[test]
fn end_removes_the_resume_target_and_a_replacement_cannot_inherit_the_uuid() {
    let rig = rig("s3reuse");
    let caller = dispatcher(&rig);
    let old = create(&rig, TARGET);
    let answer = rig.scratch.join("end-answer");
    fs::write(&answer, b"y\n").expect("End answer for the private session");
    let mut end = command(&rig, None, &["end", TARGET, "--keep-history"]);
    end.stdin(fs::File::open(answer).expect("answer on stdin"));
    succeeds(&output(&mut end));
    assert!(rig.home.join("archive").join(&old).is_dir());
    assert!(!rig.dir(TARGET).exists());

    let pane = Some(caller.pane.as_str());
    refuses(&rig, pane, &resume(TARGET, &old));
    let new = create(&rig, TARGET);
    assert_ne!(new, old);
    let out = refuses(&rig, pane, &stop(TARGET, &old));
    assert!(out.2.contains(&new), "{out:?}");
    succeeds(&run(&rig, None, &["stop", TARGET]));
    let out = refuses(&rig, pane, &resume(TARGET, &old));
    assert!(out.2.contains(&new), "{out:?}");
    succeeds(&run(&rig, pane, &resume(TARGET, &new)));
    assert_eq!(census_uuid(&rig, TARGET), new);
}

#[test]
fn matching_record_cannot_stop_a_live_tmux_session_with_a_different_stamp() {
    let rig = rig("s3stamp");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    assert_ne!(uuid, STALE);
    let (_, id) = rig.tmux(&["display-message", "-p", "-t", TARGET, "#{session_id}"]);
    assert!(id.starts_with('$'), "{id}");
    assert!(
        rig.tmux(&["set-option", "-t", id.trim(), "@ae_session_uuid", STALE])
            .0
    );
    let out = refuses(&rig, Some(&caller.pane), &stop(TARGET, &uuid));
    assert_eq!(out.2.lines().count(), 1, "{out:?}");
    assert!(out.2.contains(STALE), "missing live identity: {out:?}");
}

#[test]
fn bound_resume_rebuilds_on_the_recorded_server() {
    let mut rig = rig("s3server");
    let _caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    succeeds(&run(&rig, None, &["stop", TARGET]));
    let proposed = rig.scratch.join("second.sock");
    rig.scratch.add_tmux_server(proposed.clone());
    assert!(
        rig.keeper_on(
            &proposed,
            &[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "other",
                "sleep",
                "600"
            ]
        )
        .0
    );

    let mut cmd = command(&rig, None, &resume(TARGET, &uuid));
    cmd.env("AE_TMUX_SERVER", &proposed);
    succeeds(&output(&mut cmd));
    assert!(live(&rig, TARGET));
    let (ok, listed) = rig.tmux_on(&proposed, &["list-sessions", "-F", "#{session_name}"]);
    assert!(ok, "{listed}");
    assert_eq!(
        listed.trim(),
        "other",
        "resume moved to the proposed server"
    );
}

#[test]
fn bound_resume_refuses_a_legacy_record_without_a_server() {
    let rig = rig("s3legacy");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    succeeds(&run(&rig, None, &["stop", TARGET]));
    let path = rig.dir(TARGET).join("meta");
    let full = fs::read_to_string(&path).expect("stopped session meta");
    let mut legacy = String::new();
    for line in full.lines() {
        if !line.starts_with("tmux_server=") && !line.starts_with("tmux_server_kind=") {
            legacy.push_str(line);
            legacy.push('\n');
        }
    }
    assert_ne!(legacy, full);
    fs::write(path, legacy).expect("legacy record with no server rows");
    let out = refuses(&rig, Some(&caller.pane), &resume(TARGET, &uuid));
    assert!(out.2.contains("server"), "{out:?}");
}

fn seat_pane(rig: &Rig, seat: &str) -> String {
    let (ok, panes) = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        TARGET,
        "-F",
        "#{@ae_agent}|#{pane_id}",
    ]);
    assert!(ok, "{panes}");
    panes
        .lines()
        .find_map(|row| {
            let (actor, pane) = row.split_once('|')?;
            (actor == seat).then_some(pane)
        })
        .expect("the named seat has a physical pane")
        .to_owned()
}

fn declare(rig: &Rig, seat: &str, state: &str) {
    let pane = seat_pane(rig, seat);
    let dir = rig.dir(TARGET);
    let mut argv = vec![
        ae::cli::STATE,
        dir.to_str().expect("target state path"),
        state,
    ];
    if state == "waiting-agent" {
        argv.extend(["--on", "lead"]);
    }
    argv.push(REASON);
    let out = run(rig, Some(&pane), &argv);
    succeeds(&out);
    assert!(out.1.contains(&format!("Marked {seat} {state}")), "{out:?}");
}

#[test]
fn bound_stop_checks_every_seat_and_uses_the_latest_declaration() {
    let rig = rig("s3decl");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    let argv = stop(TARGET, &uuid);
    let pane = Some(caller.pane.as_str());
    for state in ["working", "waiting-agent", "blocked"] {
        declare(&rig, "colead", state);
        let out = refuses(&rig, pane, &argv);
        assert!(out.2.contains("colead") && out.2.contains(state), "{out:?}");
    }
    declare(&rig, "colead", "working");
    declare(&rig, "lead", "done");
    let out = refuses(&rig, pane, &argv);
    assert!(
        out.2.contains("colead") && out.2.contains("working"),
        "{out:?}"
    );
    declare(&rig, "colead", "done");
    succeeds(&run(&rig, pane, &argv));
    succeeds(&run(&rig, pane, &resume(TARGET, &uuid)));
    declare(&rig, "colead", "blocked");
    declare(&rig, "colead", "waiting-user");
    succeeds(&run(&rig, pane, &argv));
    assert!(!live(&rig, TARGET));
}

#[test]
fn bound_stop_checks_the_latest_declaration_of_a_spawned_seat() {
    let rig = rig("s3spawn");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    let dir = rig.dir(TARGET);
    succeeds(&run(
        &rig,
        Some(&seat_pane(&rig, "lead")),
        &[
            ae::cli::SPAWN,
            dir.to_str().expect("target state path"),
            "builder",
            "--using",
            "idle",
        ],
    ));
    declare(&rig, "builder", "working");
    let pane = Some(caller.pane.as_str());
    let argv = stop(TARGET, &uuid);
    let out = refuses(&rig, pane, &argv);
    assert!(
        out.2.contains("builder") && out.2.contains("working"),
        "{out:?}"
    );
    declare(&rig, "builder", "done");
    succeeds(&run(&rig, pane, &argv));
}

#[test]
fn bound_stop_refuses_an_unrecognized_latest_declaration() {
    let rig = rig("s3unknown");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    declare(&rig, "colead", "done");
    let mut journal = fs::OpenOptions::new()
        .append(true)
        .open(rig.dir(TARGET).join("events.jsonl"))
        .expect("open the private journal for a future state value");
    journal
        .write_all(concat!(
            r#"{"ts":"2026-11-19T17:00:00Z","actor":"colead","action":"state","ref":"paused","summary":"unknown state"}"#,
            "\n",
        ).as_bytes())
        .expect("append the unknown state value");
    let out = refuses(&rig, Some(&caller.pane), &stop(TARGET, &uuid));
    assert!(
        out.2.contains("colead") && out.2.contains("paused"),
        "{out:?}"
    );
}

#[test]
fn an_unreadable_declaration_source_refuses_a_bound_stop() {
    let rig = rig("s3badstate");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    let dir = rig.dir(TARGET);
    fs::rename(dir.join("events.jsonl"), dir.join("saved-events"))
        .expect("preserve the real journal");
    fs::create_dir(dir.join("events.jsonl")).expect("an unreadable journal container");
    let out = refuses(&rig, Some(&caller.pane), &stop(TARGET, &uuid));
    let diagnostic = out.2.to_ascii_lowercase();
    assert!(
        diagnostic.contains("events.jsonl") || diagnostic.contains("journal"),
        "{out:?}"
    );
}

#[test]
fn a_held_lifecycle_lock_keeps_bound_stop_from_writing_or_killing() {
    let rig = rig("s3held");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    let path = rig.home.join(format!("sessions/.lifecycle.{TARGET}.lock"));
    let held =
        ae::store::lock(&path, Duration::ZERO).expect("fixture holds the actual lifecycle lock");
    let out = refuses(&rig, Some(&caller.pane), &stop(TARGET, &uuid));
    assert!(
        out.2.contains("lock") || out.2.contains("progress"),
        "{out:?}"
    );
    drop(held);
    succeeds(&run(&rig, Some(&caller.pane), &stop(TARGET, &uuid)));
}

#[test]
fn a_held_declaration_lock_keeps_bound_stop_from_reading_past_an_inflight_writer() {
    let rig = rig("s3journal");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    let path = rig.dir(TARGET).join("events.jsonl.lock");
    let held = ae::store::lock(&path, Duration::ZERO).expect("fixture holds the real journal lock");
    refuses(&rig, Some(&caller.pane), &stop(TARGET, &uuid));
    drop(held);
    declare(&rig, "colead", "working");
    let out = refuses(&rig, Some(&caller.pane), &stop(TARGET, &uuid));
    assert!(
        out.2.contains("colead") && out.2.contains("working"),
        "{out:?}"
    );
}

#[test]
fn bound_stop_refuses_the_callers_own_session() {
    let rig = rig("s3self");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    let (ok, pane) = rig.tmux(&["display-message", "-p", "-t", TARGET, "#{pane_id}"]);
    assert!(ok && pane.starts_with('%'), "{pane}");
    let out = refuses(&rig, Some(pane.trim()), &stop(TARGET, &uuid));
    assert_eq!(out.2.lines().count(), 1, "{out:?}");
    assert!(out.2.contains("--self"), "{out:?}");

    // A selected foreign pane cannot turn the caller's own session into
    // another session or make the dispatcher bypass the bound grammar.
    let orch = census_uuid(&rig, "orch");
    let other = Some(caller.pane.as_str());
    let raw = refuses(
        &rig,
        other,
        &[ae::cli::STOP, "orch", "--expect-uuid", &orch],
    );
    assert!(raw.2.contains("--self"), "{raw:?}");
    let argv = [stop("orch", &orch).as_slice(), &["--pane", pane.trim()]].concat();
    refuses(&rig, other, &argv);
    let argv = [stop(TARGET, &uuid).as_slice(), &["--pane", pane.trim()]].concat();
    refuses(&rig, other, &argv);
    refuses(&rig, None, &argv);
    let duplicate = [
        stop(TARGET, &uuid).as_slice(),
        &["--pane", &caller.pane, "--pane", &caller.pane],
    ]
    .concat();
    refuses(&rig, other, &duplicate);
}

fn dispatcher_refuses(rig: &Rig, caller: &Seat, argv: &[&str]) {
    let out = refuses(rig, Some(&caller.pane), argv);
    assert!(
        out.2.contains("orchestrator") && out.2.contains("REFUSED"),
        "{argv:?}: {out:?}"
    );
}

#[test]
fn dispatcher_refuses_plain_lifecycle_and_bound_stop_with_extra_authority() {
    let rig = rig("s3stopargv");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    for argv in [
        vec!["stop", TARGET],
        vec!["stop", TARGET, "-y"],
        vec!["stop", "all"],
        vec!["end", TARGET],
        vec!["end", TARGET, "-f"],
        vec!["end", TARGET, "--force"],
        vec!["end", TARGET, "--purge-history"],
        vec!["rm", TARGET],
    ] {
        dispatcher_refuses(&rig, &caller, &argv);
    }
    for extra in [
        "-y",
        "--force",
        "--self",
        "--supervise",
        "--handoff",
        "--purge-history",
    ] {
        let argv = [stop(TARGET, &uuid).as_slice(), &[extra]].concat();
        refuses(&rig, Some(&caller.pane), &argv);
    }
    let duplicate = [stop(TARGET, &uuid).as_slice(), &["--expect-uuid", &uuid]].concat();
    refuses(&rig, Some(&caller.pane), &duplicate);
    refuses(&rig, Some(&caller.pane), &["stop", TARGET, "--expect-uuid"]);
    let all = create(&rig, "all");
    refuses(&rig, Some(&caller.pane), &stop("all", &all));
    let out = refuses(&rig, None, &stop("all", &all));
    assert_eq!(out.2.lines().count(), 1, "{out:?}");
    assert!(out.2.contains("all"), "{out:?}");
    assert!(live(&rig, "all") && live(&rig, TARGET));
}

#[test]
fn dispatcher_refuses_noncanonical_launches_and_the_direct_served_entry() {
    let rig = rig("s3runargv");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    succeeds(&run(&rig, None, &["stop", TARGET]));
    for argv in [
        vec![TARGET, "--resume", "--no-attach"],
        vec![TARGET, "--no-attach"],
        vec!["fresh", "--no-attach"],
        vec!["fresh", "--new", "--no-attach"],
    ] {
        dispatcher_refuses(&rig, &caller, &argv);
    }
    for argv in [
        vec![TARGET, "--resume", "--expect-uuid", &uuid],
        vec![TARGET, "--no-attach", "--expect-uuid", &uuid],
        vec![TARGET, "--resume", "--no-attach", "--expect-uuid"],
    ] {
        refuses(&rig, Some(&caller.pane), &argv);
    }
    for extra in ["--local", "--solo", "--resume", "--no-attach", "--force"] {
        let argv = [resume(TARGET, &uuid).as_slice(), &[extra]].concat();
        refuses(&rig, Some(&caller.pane), &argv);
    }
    let argv = [
        &[
            ae::cli::LAUNCH,
            "--home",
            rig.home.to_str().expect("home path"),
            "--cwd",
            rig.project.to_str().expect("project path"),
            "--global",
            rig.config.to_str().expect("global config path"),
            "--server-kind",
            "socket",
            "--server",
            rig.sock.to_str().expect("server path"),
            "--no-attach",
            "--",
        ][..],
        &resume(TARGET, &uuid),
    ]
    .concat();
    dispatcher_refuses(&rig, &caller, &argv);
    succeeds(&run(&rig, Some(&caller.pane), &resume(TARGET, &uuid)));
}

#[test]
fn dispatcher_refuses_the_orchestrator_seat_picker_and_settings_routes() {
    let rig = rig("s3roleargv");
    let caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    succeeds(&run(&rig, None, &["stop", TARGET]));
    dispatcher_refuses(&rig, &caller, &["orchestrator", "--no-attach"]);
    dispatcher_refuses(&rig, &caller, &resume("orchestrator", &uuid));
    let deadline = (ae::time::Timestamp::now().epoch() + 60).to_string();
    dispatcher_refuses(
        &rig,
        &caller,
        &[
            "orchestrator",
            "--picker-resume",
            TARGET,
            "--client",
            "/dev/pts/999",
            "--client-pid",
            "1",
            "--server-pid",
            "1",
            "--server-start",
            "1",
            "--deadline",
            &deadline,
        ],
    );
    dispatcher_refuses(
        &rig,
        &caller,
        &[
            "orchestrator",
            "--settings-apply",
            "start",
            "--target",
            "orchestrator",
            "--uuid",
            "",
            "--unattended",
            "--deadline",
            &deadline,
        ],
    );
}

#[test]
fn plain_human_lifecycle_keeps_its_existing_behavior() {
    let rig = rig("s3human");
    let _caller = dispatcher(&rig);
    let uuid = create(&rig, TARGET);
    declare(&rig, "colead", "working");
    succeeds(&run(&rig, None, &["stop", TARGET]));
    assert!(!live(&rig, TARGET));
    succeeds(&run(&rig, None, &[TARGET, "--resume", "--no-attach"]));
    assert!(live(&rig, TARGET));
    succeeds(&run(&rig, None, &[TARGET, "--resume", "--no-attach"]));
    assert_eq!(census_uuid(&rig, TARGET), uuid);
}
