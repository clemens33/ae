//! Frozen #287 slice 3: real `_run`, fixture executables, journal facts.
//! Oracle: approved plan, Slice 3 and Original start. No production mocks.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect only their own files and child processes"
)]

use super::*;
use ae::birth::{self, Record};
use ae::events::Event;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const BIRTH: &str = "aaaaaaaa-aaaa-4aaa-8aaa-000000000001";
const RESEAT: &str = "aaaaaaaa-aaaa-4aaa-8aaa-000000000002";
const SLOT: &str = "spawned.0";

fn worker(tag: &str) -> Rig {
    let rig = Rig::new(tag);
    rig.seat("claude", "");
    for row in [
        format!("session_id={SID}"),
        "seat.spawned.0=scout".into(),
        "profile.spawned.0=claude".into(),
        "agent_bin.spawned.0=claude".into(),
        format!("launch_id.spawned.0={BIRTH}"),
    ] {
        rig.append_meta(&row);
    }
    rig
}

fn run(rig: &Rig, slot: &str, print: bool, snapshot: bool) -> std::process::Output {
    let mut command = ae();
    command
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env("HOME", &rig.home)
        .current_dir(&rig.project)
        .arg(ae::cli::RUN);
    if print {
        command.arg("--print");
    }
    if snapshot {
        command
            .arg("--command-snapshot")
            .arg(format!("{} --flag", rig.tool("claude")));
    }
    command.arg(&rig.dir).arg(slot).output().expect("real _run")
}

fn journal(rig: &Rig) -> String {
    std::fs::read_to_string(rig.dir.join("events.jsonl")).unwrap_or_default()
}

fn facts(text: &str) -> Vec<birth::Fact> {
    text.lines()
        .map(|line| Event::parse_line(line).expect("actual journal event"))
        .filter(|event| event.action == birth::FACT)
        .map(|event| match birth::decode(&event) {
            Some(Ok(Record::Fact(fact))) => fact,
            other => panic!("writer emitted an untyped fact: {other:?}"),
        })
        .collect()
}

fn kinds(facts: &[birth::Fact]) -> Vec<&str> {
    facts.iter().map(|fact| fact.kind.as_str()).collect()
}

fn assert_start(fact: &birth::Fact, launch: &str) {
    assert_eq!(fact.session.as_deref(), Some(SID));
    assert_eq!(fact.birth, launch);
    let key = fact.start.as_deref().expect("start fact carries a key");
    let prefix = format!("{SLOT}:{launch}:");
    let pid = key.strip_prefix(&prefix).expect("slot and launch id");
    assert!(pid.parse::<u32>().is_ok_and(|pid| pid > 0), "{key}");
    assert!(fact.observed_source.is_none());
    assert!(fact.observed_model.is_none());
    assert!(fact.observed_effort.is_none());
}

#[test]
fn stats3_create_fact_exists_when_the_real_fixture_exec_begins_in_both_run_forms() {
    for snapshot in [false, true] {
        let rig = worker(if snapshot { "s3snapshot" } else { "s3create" });
        let tool = rig.bin.join("claude");
        let body = std::fs::read_to_string(&tool).expect("fixture executable");
        let at_exec = rig.scratch.join("journal-at-exec");
        let copy = format!(
            "cat {} > {}\n",
            ae::launch::shell_quote(&rig.dir.join("events.jsonl").display().to_string()),
            ae::launch::shell_quote(&at_exec.display().to_string()),
        );
        std::fs::write(
            &tool,
            body.replacen("#!/bin/sh\n", &format!("#!/bin/sh\n{copy}"), 1),
        )
        .expect("fixture snapshots journal on entry");
        let output = run(&rig, SLOT, false, snapshot);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(rig.out.is_file(), "real fixture tool executed");
        let at_exec = std::fs::read_to_string(at_exec).expect("journal seen by the tool");
        let seen = facts(&at_exec);
        assert_eq!(kinds(&seen), ["start-attempted"]);
        assert_start(&seen[0], BIRTH);
        assert_eq!(
            facts(&journal(&rig)),
            seen,
            "a returned tool exit is not exec failure"
        );
        assert!(
            !at_exec.contains("worker-birth"),
            "the child does not create a birth"
        );
    }
}

#[test]
fn stats3_returned_exec_records_failure_for_the_same_start_and_removes_the_marker() {
    let rig = worker("s3execfail");
    std::fs::write(rig.bin.join("claude"), "#!/missing-stats3-interpreter\n")
        .expect("executable exists but exec cannot load its interpreter");
    let output = run(&rig, SLOT, false, false);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("could not start"), "{stderr}");
    assert!(!rig.out.exists(), "fixture tool never ran");
    let seen = facts(&journal(&rig));
    assert_eq!(kinds(&seen), ["start-attempted", "start-failed"]);
    for fact in &seen {
        assert_start(fact, BIRTH);
    }
    assert_eq!(seen[0].start, seen[1].start);
    assert!(!ae::run::started_marker(&rig.dir, SLOT).exists());
}

#[test]
fn stats3_attempt_append_failure_is_loud_and_the_fixture_tool_still_executes() {
    let rig = worker("s3attemptlost");
    std::fs::create_dir(rig.dir.join("events.jsonl")).expect("journal append failure");
    let output = run(&rig, SLOT, false, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        rig.out.is_file(),
        "the launch proceeds despite the fact append failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("start-attempted"),
        "identify the lost fact: {stderr}"
    );
    assert!(
        stderr.contains("journal"),
        "say the append failed: {stderr}"
    );
}

#[test]
fn stats3_failed_append_of_both_start_facts_preserves_the_real_exec_failure() {
    let rig = worker("s3failurelost");
    std::fs::create_dir(rig.dir.join("events.jsonl")).expect("journal append failure");
    std::fs::write(rig.bin.join("claude"), "#!/missing-stats3-interpreter\n")
        .expect("fixture exec failure");
    let output = run(&rig, SLOT, false, false);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    for fact in ["start-attempted", "start-failed"] {
        assert!(
            stderr.contains(fact),
            "identify each lost fact {fact}: {stderr}"
        );
    }
    assert!(
        stderr.contains("could not start"),
        "exec was attempted: {stderr}"
    );
    assert!(!ae::run::started_marker(&rig.dir, SLOT).exists());
}

#[test]
fn stats3_print_fixed_seat_and_resume_write_no_worker_start_fact() {
    let rig = worker("s3inertmodes");
    let output = run(&rig, SLOT, true, false);
    assert!(output.status.success());
    assert!(facts(&journal(&rig)).is_empty());
    assert!(!rig.out.exists());
    let output = run(&rig, "main", false, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(rig.out.is_file());
    assert!(
        facts(&journal(&rig)).is_empty(),
        "fixed seats have no worker birth"
    );
    std::fs::write(ae::run::started_marker(&rig.dir, SLOT), "").expect("existing worker start");
    let output = run(&rig, SLOT, false, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        facts(&journal(&rig)).is_empty(),
        "a resume fallback is not a Create"
    );
}

#[test]
fn stats3_a_refusal_before_exec_writes_no_attempt_and_runs_no_tool() {
    let rig = worker("s3refusal");
    rig.append_meta(&format!(
        "config_home.{SLOT}={}",
        std::fs::canonicalize(rig.home.join(".claude"))
            .expect("canonical fixture config home")
            .display()
    ));
    rig.chmod(0o555);
    let output = run(&rig, SLOT, false, false);
    rig.chmod(0o755);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("refusing to launch"),
        "the start marker must refuse before exec: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!rig.out.exists());
    assert!(
        facts(&journal(&rig)).is_empty(),
        "pre-exec refusal is not an exec attempt"
    );
}

#[test]
fn stats3_reseat_create_names_its_new_launch_id_instead_of_the_original_birth() {
    let rig = worker("s3reseat");
    let path = rig.dir.join("meta");
    let meta = std::fs::read_to_string(&path).expect("fixture meta");
    std::fs::write(path, meta.replace(BIRTH, RESEAT)).expect("reseat rotates launch id");
    let output = run(&rig, SLOT, false, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let seen = facts(&journal(&rig));
    assert_eq!(kinds(&seen), ["start-attempted"]);
    assert_start(&seen[0], RESEAT);
    assert_ne!(
        seen[0].birth, BIRTH,
        "the fold cannot staple a reseat to the original birth"
    );
}
