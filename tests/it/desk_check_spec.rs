//! Frozen Deskcheck P1 acceptance through the normal helper routes.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the fixture runs the real helper and owns an isolated fleet"
)]

use ae::telegram::{
    Echoes,
    bridge::SharedChat,
    inbound::{Origin, Submit, Submitted},
    routing::Target,
};
use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use super::cli::{Runner, ae, bounded, byte_tree, helper};
use super::o2guard_p2_spec::uuid_of;
use super::relay_chat_spec::{Out, Rig, echo_count, receipt, words};

const SECTIONS: [&str; 4] = [
    "unboarded",
    "boarded (hint)",
    "known closed",
    "unresolved coverage",
];

fn rig(tag: &str) -> Rig {
    let rig = Rig::dispatcher(tag);
    for session in ["orch", rig.owner.home.as_str()] {
        rig.owner
            .tmux(&["set-environment", "-t", session, "AE_SESSION", "1"]);
    }
    rig
}

fn run(rig: &Rig, mut command: Runner, pane: Option<&str>, args: &[&str]) -> Out {
    let root = &rig.world.root;
    let socket = &rig.world.sock;
    command
        .env("HOME", root)
        .env("AE_HOME", root)
        .env("CONFIG_FILE", root.join("config"))
        .env("TMUX_TMPDIR", root)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", socket)
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(pane) = pane {
        command
            .env("TMUX", format!("{},0,0", socket.display()))
            .env("TMUX_PANE", pane);
    }
    let child = command.spawn().expect("run ae in the private fleet");
    let out =
        bounded(child, Duration::from_secs(12)).expect("helper returns within the fixture bound");
    (
        out.status.code(),
        String::from_utf8(out.stdout).expect("stdout is text"),
        String::from_utf8(out.stderr).expect("stderr is text"),
    )
}

fn report(rig: &Rig, all: bool) -> Out {
    let mut args = vec!["@orch", "desk", "check"];
    if all {
        args.push("--all");
    }
    run(rig, ae(), None, &args)
}

fn success(out: &Out, code: i32) {
    assert_eq!(out.0, Some(code), "{}", words(out));
}

fn section<'a>(out: &'a Out, name: &str) -> Vec<&'a str> {
    let mut lines = out.1.lines();
    assert!(
        lines
            .by_ref()
            .any(|line| line.trim().to_ascii_lowercase().starts_with(name)),
        "no {name} section in {}",
        words(out)
    );
    lines
        .take_while(|line| {
            let line = line.trim().to_ascii_lowercase();
            !SECTIONS.iter().any(|heading| line.starts_with(heading))
        })
        .collect()
}

fn row<'a>(out: &'a Out, name: &str, id: &str) -> &'a str {
    section(out, name)
        .into_iter()
        .find(|line| line.contains(id))
        .unwrap_or_else(|| panic!("missing {id} in {name}: {}", words(out)))
}

fn excludes(out: &Out, name: &str, id: &str) {
    assert!(
        section(out, name)
            .into_iter()
            .all(|line| !line.contains(id)),
        "unexpected {id} in {name}: {}",
        words(out)
    );
}

fn board(rig: &Rig, item: &str) {
    fs::write(
        rig.world.root.join("board.txt"),
        format!("RUNNING\n{item}\n"),
    )
    .expect("provide board in its ordinary input grammar");
    let out = run(
        rig,
        ae(),
        Some(&rig.caller.pane),
        &["@orch", "desk", "set", "board.txt"],
    );
    success(&out, 0);
}

fn waiting(rig: &Rig) {
    let route = format!("@{}", rig.owner.home);
    let out = run(
        rig,
        ae(),
        Some(&rig.owner.tool.pane),
        &[
            &route,
            "state",
            "waiting-user",
            "The private fixture needs the owner's input before continuing with the next verification step.",
        ],
    );
    success(&out, 0);
}

fn owner_input(rig: &Rig, update_id: i64) -> String {
    let chat = SharedChat {
        root: rig.world.root.clone(),
        echoes: Echoes::default(),
    };
    let target = Target {
        session: "orch".to_owned(),
        dir: rig.caller.dir.clone(),
        session_id: uuid_of("orch"),
        pair: vec!["boss".to_owned()],
        seat: Some("boss".to_owned()),
    };
    let origin = Origin {
        update_id,
        date: ae::time::Timestamp::now().epoch(),
        from_id: "42".to_owned(),
    };
    let result = chat.submit(&target, "Please verify the fixture.", &origin);
    assert!(
        matches!(
            result,
            Submitted::Sent { .. } | Submitted::Unconfirmed { .. }
        ),
        "private Telegram ingress: {result:?}"
    );
    origin.request_id()
}

fn bound_relay(rig: &Rig, input: &str) -> String {
    let out = rig.run(&[
        &rig.owner.home,
        "--owner-input",
        input,
        "--task",
        "Verify this fixture.",
    ]);
    success(&out, 0);
    let id = out.1.lines().next().expect("relay id");
    receipt(&out, id, "sent", "pending");
    id.to_owned()
}

fn owner_reply(rig: &Rig, input: &str) {
    let out = run(
        rig,
        ae(),
        Some(&rig.caller.pane),
        &[
            "@orch",
            "reply",
            input,
            "The owner-facing request now has a reply.",
        ],
    );
    success(&out, 0);
}

fn same_bytes_and_mtimes(root: &Path) -> Vec<(String, Option<Vec<u8>>, std::time::SystemTime)> {
    byte_tree(root)
        .into_iter()
        .map(|(path, bytes)| {
            let mtime = fs::metadata(root.join(&path))
                .and_then(|meta| meta.modified())
                .expect("snapshot modification time");
            (path, bytes, mtime)
        })
        .collect()
}

#[test]
fn the_helper_and_short_form_report_an_absent_board_and_current_wait() {
    let rig = rig("dckwait");
    let meta_path = rig.owner.tool.dir.join("meta");
    let original_meta = fs::read_to_string(&meta_path).expect("seat metadata");
    let started = ae::time::Timestamp::now().epoch() - 10;
    let meta = format!("{original_meta}launch_time.main={started}\n");
    fs::write(&meta_path, &meta).expect("a live seat has a completed launch baseline");
    waiting(&rig);
    let seat = format!("{}:lead", rig.owner.home);
    let out = report(&rig, false);
    success(&out, 1);
    assert_eq!(out.1.lines().next(), Some("board: absent"));
    let wait = row(&out, "unboarded", &seat);
    let uuid = meta
        .lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .expect("UUID");
    assert!(wait.contains(uuid) && wait.contains("tok-rig"), "{wait}");

    let link = rig.caller.dir.join("desk");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), &link).expect("private desk link");
    let link_result = run(&rig, helper(&link), None, &["check"]);
    success(&link_result, 1);
    row(&link_result, "unboarded", &seat);

    board(
        &rig,
        &format!("R1 | Similar {seat}-backup and prefix-{seat}"),
    );
    let out = report(&rig, false);
    success(&out, 1);
    row(&out, "unboarded", &seat);
    excludes(&out, "boarded (hint)", &seat);

    board(
        &rig,
        &format!("R1 | Verify the live seat | https://example.test/desk/{seat}"),
    );
    let out = report(&rig, false);
    success(&out, 0);
    excludes(&out, "unboarded", &seat);
    let hint = row(&out, "boarded (hint)", &seat);
    assert!(hint.contains("R1") && hint.contains("link"), "{hint}");

    // Retain the old declaration across a new launch of the same named seat.
    std::thread::sleep(Duration::from_secs(1));
    let replacement_started = ae::time::Timestamp::now().epoch();
    let replacement_meta = original_meta.replace("tok-rig", "tok-new");
    fs::write(
        &meta_path,
        format!("{replacement_meta}launch_time.main={replacement_started}\n"),
    )
    .expect("record the completed start of a replacement seat launch");
    let out = report(&rig, false);
    success(&out, 0);
    excludes(&out, "boarded (hint)", &seat);
    excludes(&out, "unboarded", &seat);
    std::thread::sleep(Duration::from_secs(1));
    waiting(&rig);
    let out = report(&rig, false);
    success(&out, 0);
    assert!(row(&out, "boarded (hint)", &seat).contains("tok-new"));
}

#[test]
fn owner_and_two_bound_relays_follow_their_independent_request_states_without_repair() {
    let rig = rig("dckowner");
    let input = owner_input(&rig, 13);
    let first = bound_relay(&rig, &input);
    let second = bound_relay(&rig, &input);
    let out = report(&rig, false);
    success(&out, 1);
    let owner = row(&out, "unboarded", &input);
    assert!(owner.contains(&first) && owner.contains(&second), "{owner}");
    row(&out, "unboarded", &first);
    row(&out, "unboarded", &second);

    board(&rig, &format!("R2 | Waiting on ({input})"));
    let out = report(&rig, false);
    success(&out, 1);
    excludes(&out, "unboarded", &input);
    let hint = row(&out, "boarded (hint)", &input);
    assert!(hint.contains("R2") && hint.contains("text"), "{hint}");
    row(&out, "unboarded", &first);
    row(&out, "unboarded", &second);

    owner_reply(&rig, &input);
    rig.reply(&second, "A target reply before the orchestrator echo.");
    let echos = echo_count(&rig, &second);
    assert!(echos <= 1, "fixture has no orchestrator reply echo yet");
    let board_lock = rig.caller.dir.join("desk-record.lock");
    let journal_lock = rig.caller.dir.join("events.jsonl.lock");
    let held_board =
        ae::store::lock(&board_lock, Duration::ZERO).expect("fixture holds board writer");
    let held_journal =
        ae::store::lock(&journal_lock, Duration::ZERO).expect("fixture holds journal writer");
    let root = rig.world.root.join("sessions");
    let before = same_bytes_and_mtimes(&root);
    let out = report(&rig, false);
    success(&out, 1);
    excludes(&out, "unboarded", &input);
    row(&out, "unboarded", &first);
    excludes(&out, "unboarded", &second);
    row(&out, "known closed", &second);
    assert_eq!(echo_count(&rig, &second), echos);
    assert_eq!(same_bytes_and_mtimes(&root), before);
    drop(held_journal);
    drop(held_board);

    // The opposite order is independent: a completed delegation does not answer the owner.
    let other = owner_input(&rig, 14);
    let relay = bound_relay(&rig, &other);
    rig.reply(&relay, "The second owner still needs a direct reply.");
    let out = report(&rig, false);
    let owner = row(&out, "unboarded", &other);
    assert!(
        owner.contains(&relay) && owner.contains("replied"),
        "{owner}"
    );
    row(&out, "known closed", &relay);
}

#[test]
fn a_renamed_target_follows_its_uuid_and_a_stopped_or_missing_target_is_unresolved() {
    let mut rig = rig("dckrename");
    let id = rig.ask("Answer this relay before the target moves.");
    rig.reply(&id, "Target reply before any echo.");
    let old = rig.owner.home.clone();
    let moved = "deskcheck-moved";
    rig.owner.tmux(&["rename-session", "-t", &old, moved]);
    let dir = rig.owner.tool.dir.with_file_name(moved);
    fs::rename(&rig.owner.tool.dir, &dir).expect("move the target state alongside the pane");
    let path = dir.join("meta");
    let meta = fs::read_to_string(&path).expect("target metadata");
    fs::write(
        &path,
        meta.replace(&format!("session={old}\n"), &format!("session={moved}\n")),
    )
    .expect("rename preserves target uuid");
    let replacement = rig.world.session(&old, "lead", "");
    rig.owner
        .tmux(&["set-environment", "-t", &old, "AE_SESSION", "1"]);
    rig.owner.tool.dir = dir;
    rig.owner.home = moved.to_owned();
    let out = report(&rig, false);
    success(&out, 0);
    row(&out, "known closed", &id);
    excludes(&out, "unresolved coverage", &id);
    assert_ne!(replacement.dir, rig.owner.tool.dir);

    rig.owner.tmux(&["kill-session", "-t", moved]);
    let out = report(&rig, false);
    success(&out, 0);
    let gap = row(&out, "unresolved coverage", &id);
    assert!(gap.to_ascii_lowercase().contains("stopped"), "{gap}");
    excludes(&out, "known closed", &id);

    fs::rename(&rig.owner.tool.dir, rig.world.root.join("purged-target"))
        .expect("simulate a purged record while keeping the same-name replacement");
    let out = report(&rig, false);
    success(&out, 0);
    row(&out, "unresolved coverage", &id);
    excludes(&out, "known closed", &id);
}

#[test]
fn target_read_errors_and_board_read_errors_have_distinct_exit_codes() {
    let rig = rig("dckbad");
    let id = rig.ask("This relay's journal will become unreadable.");
    let journal = rig.owner.tool.dir.join("events.jsonl");
    fs::remove_file(&journal).expect("remove readable fixture journal");
    fs::create_dir(&journal).expect("a directory cannot be read as a journal");
    let out = report(&rig, false);
    success(&out, 0);
    row(&out, "unresolved coverage", &id);
    let gap = section(&out, "unresolved coverage")
        .into_iter()
        .find(|line| line.contains(&rig.owner.home) && !line.contains(&id))
        .expect("the unreadable session has its own coverage row");
    let normalized = gap.to_ascii_lowercase();
    assert!(
        normalized.contains("journal") || normalized.contains("unreadable"),
        "{gap}"
    );
    excludes(&out, "known closed", &id);
    excludes(&out, "unboarded", &id);

    let record = rig.caller.dir.join("desk-record");
    fs::write(&record, "damaged board fixture\n").expect("unparseable board");
    let out = report(&rig, false);
    success(&out, 2);
    assert!(out.1.is_empty(), "{out:?}");
    assert_eq!(out.2.lines().count(), 1, "{out:?}");
    assert!(out.2.contains(&record.display().to_string()), "{out:?}");
    assert!(out.2.contains("not an ae-desk 1 record"), "{out:?}");
    assert_eq!(
        fs::read_to_string(record).expect("board bytes retained"),
        "damaged board fixture\n"
    );
}

#[test]
fn a_proven_undelivered_attempt_remains_an_unboarded_candidate() {
    let rig = rig("dckmiss");
    let id = rig.refused(false);
    let out = report(&rig, false);
    success(&out, 1);
    assert!(row(&out, "unboarded", &id).contains("undelivered, no opening"));
    excludes(&out, "known closed", &id);
    board(&rig, &format!("R3 | Triage {id}"));
    let out = report(&rig, false);
    success(&out, 0);
    row(&out, "boarded (hint)", &id);
    excludes(&out, "unboarded", &id);
}

#[test]
fn default_known_closed_output_is_bounded_and_all_lists_every_id() {
    let rig = rig("dcktail");
    let oldest = rig.ask("An older completed relay.");
    rig.reply(&oldest, "The old relay has an answer.");
    std::thread::sleep(Duration::from_secs(1));
    let mut newer = Vec::new();
    for idx in 0..20 {
        let id = rig.ask(&format!("Completed relay {idx}."));
        rig.reply(&id, "The relay has an answer.");
        newer.push(id);
    }

    let short = report(&rig, false);
    success(&short, 0);
    for id in &newer {
        row(&short, "known closed", id);
    }
    excludes(&short, "known closed", &oldest);
    assert!(
        short.1.contains("1 older omitted (--all lists them)"),
        "{short:?}"
    );
    let all = report(&rig, true);
    success(&all, 0);
    for out in [&short, &all] {
        let heading = out
            .1
            .lines()
            .find(|line| line.trim().to_ascii_lowercase().starts_with("known closed"))
            .expect("known closed heading");
        assert!(
            heading
                .split(|ch: char| !ch.is_ascii_digit())
                .any(|part| part == "21"),
            "{heading}"
        );
    }
    row(&all, "known closed", &oldest);
    for id in &newer {
        row(&all, "known closed", id);
    }
    assert!(!all.1.contains("older omitted"), "{all:?}");
}
