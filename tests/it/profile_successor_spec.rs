//! Independent acceptance for the stopped-seat profile successor contract.
//! Real launch, spawn, list, doctor, publication and rollback on private
//! tmux servers. Only the vendor executable and its transcript are fixtures.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance inspects the private rig and fails on missing setup"
)]

use std::time::{Duration, Instant};

use super::{Rig, ae, bare_session, helper, stop};

const SESSION: &str = "psucc";
const CODEX_ID: &str = "0199c0de-5710-4890-abcd-ef0123456789";
const VALID: &str = "[profile_successors]\nlegacy = next\nhelper-old = helper-new\n\
                     bare-old = bare-new\nworker-old = worker-new\n";

fn rig(tag: &str) -> Rig {
    let rig = Rig::new(tag, &["codex", "claude"], None);
    std::fs::create_dir(rig.scratch.join("claude-carry")).expect("alternate account");
    std::fs::create_dir(rig.scratch.join("codex-alt")).expect("alternate Codex account");
    let bin = rig.bin.display();
    let root = rig.scratch.display();
    let config = format!(
        r#"[clients]
primary = {bin}/codex
secondary = {bin}/codex config_home={root}/codex-alt
anthropic = {bin}/claude
carry = {bin}/claude config_home={root}/claude-carry
[profiles]
legacy = "primary -m gpt-6.1-sol"
next = "primary -m gpt-6.2-sol"
helper-old = "anthropic --model old-helper"
helper-new = "anthropic --model new-helper"
bare-old = "anthropic --model old-bare"
bare-new = "anthropic --model new-bare"
worker-old = "anthropic --model old-worker"
worker-new = "anthropic --model new-worker"
client-old = "primary -m client-old"
client-new = "secondary -m client-new"
route-old = "primary -m route-old"
route-new = "primary --oss -m route-new"
tool-old = "anthropic --model tool-old"
tool-new = "primary -m tool-new"
orbit-a = "primary -m orbit-a"
orbit-b = "primary -m orbit-b"
chain-a = "primary -m chain-a"
chain-b = "primary -m chain-b"
chain-c = "primary -m chain-c"
[client_pairs]
helper-old = carry
[roster]
lead = legacy
mate = worker-old
[workspace]
main = lead
layout = vertical
watchdog = false
chat = off
auto_upgrade = off
auto_reseat = off
"#
    );
    std::fs::write(&rig.config, config).expect("private config");
    rig
}

fn append(rig: &Rig, extra: &str) {
    let base = std::fs::read_to_string(&rig.config).expect("base config");
    std::fs::write(&rig.config, format!("{base}\n{extra}")).expect("new private config");
}

fn launch(rig: &Rig) -> (Option<i32>, String, String) {
    let result = rig.launch(&["--local", SESSION]);
    assert_eq!(result.0, Some(0), "{result:?}");
    result
}

fn cli(rig: &Rig, tail: &[&str]) -> (Option<i32>, String) {
    let output = ae()
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &rig.config)
        .env("TMUX_TMPDIR", &rig.scratch)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .current_dir(&rig.project)
        .args(tail)
        .output()
        .expect("isolated public CLI");
    (
        output.status.code(),
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

fn value<'a>(meta: &'a str, key: &str) -> &'a str {
    let prefix = format!("{key}=");
    let mut rows = meta.lines().filter_map(|row| row.strip_prefix(&prefix));
    let found = rows
        .next()
        .unwrap_or_else(|| panic!("missing {key}: {meta}"));
    assert!(rows.next().is_none(), "more than one {key}: {meta}");
    found
}

fn journal(rig: &Rig) -> String {
    std::fs::read_to_string(rig.dir(SESSION).join("events.jsonl")).expect("session journal")
}

fn changes(rig: &Rig) -> Vec<ae::events::Event> {
    journal(rig)
        .lines()
        .filter_map(|line| ae::events::Event::parse_line(line).ok())
        .filter(|event| event.action == "profile-successor")
        .collect()
}

fn assert_change(rig: &Rig, old: &str, next: &str) {
    let summary = format!("stored profile rewritten {old} -> {next}");
    let rows = changes(rig);
    assert_eq!(
        rows.iter()
            .filter(|event| event.summary.as_deref() == Some(summary.as_str()))
            .count(),
        1,
        "{rows:?}"
    );
}

fn wait_exec(rig: &Rig, parts: &[&str]) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let seen = std::fs::read_to_string(&rig.launched).unwrap_or_default();
        if parts.iter().all(|part| seen.contains(part)) {
            return seen;
        }
        assert!(Instant::now() < deadline, "missing {parts:?} in {seen}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn clear_receipt(rig: &Rig) {
    std::fs::write(&rig.launched, "").expect("clear only the fake harness receipt");
}

fn register_codex(rig: &Rig) {
    let meta = rig.meta(SESSION);
    let floor = value(&meta, "capture_floor.main")
        .parse()
        .expect("pre-exec capture floor");
    let token = value(&meta, "launch_id.main");
    let when = ae::time::Timestamp::from_epoch(floor).to_string();
    let store = rig
        .scratch
        .join(".codex/sessions")
        .join(when[..10].replace('-', "/"));
    std::fs::create_dir_all(&store).expect("the vendor store");
    std::fs::write(
        store.join(format!("rollout-{CODEX_ID}.jsonl")),
        format!(
            "{{\"timestamp\":\"{when}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{CODEX_ID}\",\"cwd\":\"{}\"}}}}\n\
             {{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"developer\",\"content\":[{{\"type\":\"input_text\",\"text\":\"AE_CODEX_LAUNCH_ID={token}\"}}]}}}}\n",
            rig.project.display()
        ),
    )
    .expect("synthetic rollout");
    let registered = helper(&rig.dir(SESSION).join("_register-sid"))
        .env("HOME", &rig.scratch)
        .args(["main", CODEX_ID])
        .output()
        .expect("actual Codex registration");
    assert!(registered.status.success(), "{registered:?}");
    assert_eq!(value(&rig.meta(SESSION), "harness_session.main"), CODEX_ID);
}

fn spawn(rig: &Rig, name: &str, profile: &str) {
    let spawned = helper(&rig.dir(SESSION).join("spawn"))
        .env("HOME", &rig.scratch)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args([name, "--using", profile, "--", "synthetic task"])
        .output()
        .expect("actual spawn with the old profile");
    assert!(spawned.status.success(), "{spawned:?}");
}

fn seed_claude<'a>(rig: &Rig, meta: &'a str, slot: &str, home: &std::path::Path) -> &'a str {
    let id = value(meta, &format!("harness_session.{slot}"));
    assert!(!id.is_empty() && id != "pending", "{meta}");
    let project = std::fs::canonicalize(&rig.project).expect("the canonical project cwd");
    super::plant_transcript(home, &project, id);
    id
}

fn assert_pending(rig: &Rig) {
    let (code, listing) = cli(rig, &["list", "--all"]);
    assert_eq!(code, Some(0), "{listing}");
    assert!(
        listing
            .lines()
            .any(|line| line.split_whitespace().take(2).eq([SESSION, "stopped"])),
        "the private server must prove {SESSION} stopped: {listing}"
    );
    for (name, target) in [
        ("lead", "next"),
        ("mate", "worker-new"),
        ("helper", "helper-new"),
        ("scout", "bare-new"),
    ] {
        assert!(
            listing
                .lines()
                .any(|line| line.contains(name) && line.contains(&format!("resumes on {target}"))),
            "pending migration for {name}: {listing}"
        );
    }
}

#[test]
fn profile_successor_keeps_roster_and_spawned_conversations() {
    let rig = rig("ps-good");
    append(&rig, "[workspace]\nworkers = mate\n");
    launch(&rig);
    wait_exec(&rig, &["gpt-6.1-sol", "old-worker"]);
    register_codex(&rig);
    append(&rig, VALID);
    spawn(&rig, "helper", "helper-old@carry");
    spawn(&rig, "scout", "bare-old");
    wait_exec(&rig, &["old-helper", "old-bare"]);
    let live = rig.meta(SESSION);
    let events = journal(&rig);
    let panes = rig.panes(SESSION);
    assert_eq!(value(&live, "profile.main"), "legacy");
    assert_eq!(value(&live, "profile.worker.0"), "worker-old");
    assert_eq!(value(&live, "profile.spawned.0"), "helper-old");
    assert_eq!(value(&live, "client.spawned.0"), "carry");
    assert_eq!(value(&live, "profile.spawned.1"), "bare-old");
    assert!(!live.contains("client.spawned.1="), "{live}");
    launch(&rig);
    let (code, listing) = cli(&rig, &["list", "--all"]);
    assert_eq!(code, Some(0), "{listing}");
    assert!(!listing.contains("resumes on "), "{listing}");
    assert_eq!(rig.meta(SESSION), live);
    assert_eq!(journal(&rig), events);
    assert_eq!(rig.panes(SESSION), panes);

    let store = value(&live, "config_home.spawned.0");
    // Keep the private server alive so list can prove absence after stop.
    let _keeper = bare_session(&rig, &rig.sock, "keeper");
    stop(&rig, SESSION);
    let helper_id = seed_claude(&rig, &live, "spawned.0", std::path::Path::new(store));
    let default = rig.scratch.join(".claude");
    let scout_id = seed_claude(&rig, &live, "spawned.1", &default);
    let mate_id = seed_claude(&rig, &live, "worker.0", &default);
    clear_receipt(&rig);

    assert_pending(&rig);
    assert_eq!(rig.meta(SESSION), live, "stopped list is read-only");
    assert!(changes(&rig).is_empty());

    launch(&rig);
    let seen = wait_exec(
        &rig,
        &[
            "gpt-6.2-sol",
            &format!("resume {CODEX_ID}"),
            "new-helper",
            "new-bare",
            "new-worker",
            &format!("--resume {helper_id}"),
            &format!("--resume {scout_id}"),
            &format!("--resume {mate_id}"),
            &format!("CLAUDE_CONFIG_DIR={store}"),
        ],
    );
    for old in ["gpt-6.1-sol", "old-helper", "old-bare", "old-worker"] {
        assert!(!seen.contains(old), "{seen}");
    }
    for (model, id) in [
        ("gpt-6.2-sol", CODEX_ID),
        ("new-helper", helper_id),
        ("new-bare", scout_id),
        ("new-worker", mate_id),
    ] {
        assert!(
            seen.lines()
                .any(|line| line.contains(model) && line.contains(id)),
            "{model} did not reopen {id}: {seen}"
        );
    }
    let resumed = rig.meta(SESSION);
    for (key, expected) in [
        ("profile.main", "next"),
        ("profile.worker.0", "worker-new"),
        ("profile.spawned.0", "helper-new"),
        ("profile.spawned.1", "bare-new"),
        ("client.spawned.0", "carry"),
        ("config_home.spawned.0", store),
        ("harness_session.main", CODEX_ID),
        ("harness_session.worker.0", mate_id),
        ("harness_session.spawned.0", helper_id),
        ("harness_session.spawned.1", scout_id),
    ] {
        assert_eq!(value(&resumed, key), expected, "{resumed}");
    }
    assert_eq!(changes(&rig).len(), 4);
    assert_change(&rig, "legacy", "next");
    assert_change(&rig, "helper-old", "helper-new");
    assert_change(&rig, "bare-old", "bare-new");
    assert_change(&rig, "worker-old", "worker-new");
    stop(&rig, SESSION);
    let (code, listing) = cli(&rig, &["list", "--all"]);
    assert_eq!(code, Some(0), "{listing}");
    assert!(!listing.contains("resumes on "), "{listing}");
    launch(&rig);
    assert_eq!(
        changes(&rig).len(),
        4,
        "a later resume never repeats a rewrite"
    );
}

#[test]
fn profile_successor_invalid_rows_emit_once_and_leave_the_chain_tail_usable() {
    let rig = rig("ps-invalid");
    launch(&rig);
    wait_exec(&rig, &["gpt-6.1-sol"]);
    stop(&rig, SESSION);
    let tail = "psucc-tail";
    let started = rig.launch(&["--local", tail, "--lead", "chain-b"]);
    assert_eq!(started.0, Some(0), "{started:?}");
    wait_exec(&rig, &["chain-b"]);
    assert_eq!(value(&rig.meta(tail), "profile.main"), "chain-b");
    stop(&rig, tail);
    clear_receipt(&rig);

    append(
        &rig,
        "[profile_successors]\n         legacy = missing\n         client-old = client-new\n         route-old = route-new\n         tool-old = tool-new\n         orbit-a = orbit-b\norbit-b = orbit-a\n         chain-a = chain-b\nchain-b = chain-c\n",
    );
    let refused = [
        ("legacy", "target 'missing' is not a profile"),
        ("client-old", "client or provider"),
        ("route-old", "client or provider"),
        ("tool-old", "client or provider"),
        ("orbit-a", "cycle"),
        ("orbit-b", "cycle"),
        (
            "chain-a",
            "chain longer than one hop (chain-a -> chain-b -> chain-c)",
        ),
    ];
    let (_, doctor) = cli(
        &rig,
        &["doctor", "--global", &rig.config.display().to_string()],
    );
    let (_, _, resume) = launch(&rig);
    for output in [&doctor, &resume] {
        for (source, reason) in refused {
            let marker = format!("[profile_successors] '{source}' ignored");
            let notes: Vec<_> = output
                .lines()
                .filter(|line| line.contains(&marker))
                .collect();
            assert_eq!(notes.len(), 1, "{source}: {output}");
            assert!(notes[0].contains(reason), "{source}: {notes:?}");
        }
        assert!(
            !output.contains("[profile_successors] 'chain-b' ignored"),
            "{output}"
        );
    }
    wait_exec(&rig, &["gpt-6.1-sol"]);
    assert_eq!(value(&rig.meta(SESSION), "profile.main"), "legacy");
    assert!(changes(&rig).is_empty());
    let resumed = rig.launch(&["--local", tail]);
    assert_eq!(resumed.0, Some(0), "{resumed:?}");
    wait_exec(&rig, &["chain-c"]);
    assert_eq!(value(&rig.meta(tail), "profile.main"), "chain-c");
}

#[test]
fn profile_successor_a_manifest_failure_preserves_the_published_rewrite_and_its_audit() {
    let rig = rig("ps-rollback");
    let (_, doctor) = cli(
        &rig,
        &["doctor", "--global", &rig.config.display().to_string()],
    );
    assert!(!doctor.contains("profile successors"), "{doctor}");
    launch(&rig);
    wait_exec(&rig, &["gpt-6.1-sol"]);
    stop(&rig, SESSION);
    clear_receipt(&rig);
    let original = rig.meta(SESSION);

    let (code, listing) = cli(&rig, &["list", "--all"]);
    assert_eq!(code, Some(0), "{listing}");
    assert!(!listing.contains("resumes on "), "{listing}");
    assert_eq!(rig.meta(SESSION), original);
    launch(&rig);
    wait_exec(&rig, &["gpt-6.1-sol"]);
    assert_eq!(value(&rig.meta(SESSION), "profile.main"), "legacy");
    assert!(changes(&rig).is_empty());
    stop(&rig, SESSION);
    clear_receipt(&rig);

    append(&rig, "[profile_successors]\nlegacy = next\n");
    // An explicit selection of the old profile wins even when the exact old
    // profile is the source. The following flagless attempt still migrates it.
    let explicit = rig.launch(&["--local", SESSION, "--lead", "legacy"]);
    assert_eq!(explicit.0, Some(0), "{explicit:?}");
    wait_exec(&rig, &["gpt-6.1-sol"]);
    assert_eq!(value(&rig.meta(SESSION), "profile.main"), "legacy");
    assert!(changes(&rig).is_empty());
    stop(&rig, SESSION);
    clear_receipt(&rig);

    let manifest = rig.dir(SESSION).join("workspace.md");
    std::fs::remove_file(&manifest).expect("previous workspace manifest");
    std::fs::create_dir(&manifest).expect("block only the post-publish manifest rename");

    let failed = rig.launch(&["--local", SESSION]);
    assert_eq!(failed.0, Some(1), "{failed:?}");
    assert!(
        failed.2.contains("could not write the workspace manifest"),
        "{failed:?}"
    );
    assert!(!rig.sessions().contains(&SESSION.to_owned()), "{failed:?}");
    assert!(
        std::fs::read_to_string(&rig.launched)
            .expect("fake receipt")
            .is_empty()
    );
    let rewritten = rig.meta(SESSION);
    assert_eq!(value(&rewritten, "profile.main"), "next", "{rewritten}");
    assert_eq!(changes(&rig).len(), 1);
    assert_change(&rig, "legacy", "next");

    std::fs::remove_dir(manifest).expect("unblock the retry");
    launch(&rig);
    wait_exec(&rig, &["gpt-6.2-sol"]);
    assert_eq!(
        changes(&rig).len(),
        1,
        "retry must not repeat the stored rewrite"
    );
}
