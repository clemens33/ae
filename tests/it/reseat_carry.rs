//! `ae reseat` between two ACCOUNTS of one tool: the conversation TRAVELS.
//!
//! The whole operation runs on [`super::seat_relaunch::Rig`], the fixture the
//! relaunch and reseat pins already share, with two profiles that differ in
//! nothing but the `CLAUDE_CONFIG_DIR` they name. A conversation is planted in
//! the source account by hand — SYNTHETIC bytes, never a real transcript — and
//! what the move did is read out of the target account, the meta, the pane's
//! own launch argv and the session's event log.
//!
//! What every pin here is ultimately about: after the move the seat must
//! RESUME, not restart. `run::clear_slot` deletes the start marker that decides
//! that, so the copy alone would still hand the successor a brand new
//! conversation beside the one ae just carried.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::seat_relaunch::Rig;

/// The conversation that travels.
const ID: &str = "22222222-2222-4222-8222-222222222222";

/// The transcript's bytes. Synthetic: a carry copies opaquely and never parses,
/// so the pins need bytes that are RECOGNISABLE, not bytes that are valid.
const TRANSCRIPT: &[u8] = b"{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n";

/// One account of the rig's scratch, canonical — the spelling ae records and
/// compares, and the one a copy must land under.
fn account(rig: &Rig, name: &str) -> PathBuf {
    canonical_scratch(rig).join(format!("home-{name}"))
}

/// The seat's working copy, in the spelling the pane's own `getcwd` returns, so
/// the meta row and `run::resumable`'s probe agree on one project key.
fn work_dir(rig: &Rig) -> PathBuf {
    canonical_scratch(rig).join("work")
}

/// The scratch root as `getcwd` answers it. On macOS `/tmp` IS a link, so the
/// rig's own spelling and the pane's differ — and the whole carry turns on the
/// two agreeing about one project key.
fn canonical_scratch(rig: &Rig) -> PathBuf {
    std::fs::canonicalize(&rig.scratch)
        .unwrap_or_else(|why| panic!("the scratch dir should canonicalize: {why}"))
}

fn write(path: &Path, bytes: &[u8]) {
    let parent = path
        .parent()
        .unwrap_or_else(|| panic!("{} should have a parent", path.display()));
    assert!(
        std::fs::create_dir_all(parent).is_ok(),
        "{}",
        parent.display()
    );
    assert!(std::fs::write(path, bytes).is_ok(), "{}", path.display());
}

/// The conversation's whole file set, in `home`.
fn plant_store(rig: &Rig, home: &Path) {
    let key = ae::carry::project_key(&work_dir(rig));
    let project = home.join("projects").join(&key);
    write(&project.join(format!("{ID}.jsonl")), TRANSCRIPT);
    write(
        &project.join(ID).join("tool-results").join("t1.txt"),
        b"tool",
    );
    write(&project.join("memory").join("notes.md"), b"remembered");
    write(&home.join("file-history").join(ID).join("h@v1"), b"check");
    write(&home.join("tasks").join(ID).join("1.json"), b"{}");
    assert!(
        std::fs::create_dir_all(home.join("session-env").join(ID)).is_ok(),
        "the empty sidecar dir shape ae measured"
    );
}

/// A dead claude seat on account `a`, holding [`ID`], with its store planted.
///
/// The rows are written AFTER the tool has run and been killed, so what the
/// pins read is the state this verb is handed rather than whatever `_run`'s
/// own create path happened to record.
fn dead_seat(rig: &Rig) -> String {
    rig.seat_rows("spawned.0", "scout", "claude-a", "claude");
    let pane = rig.new_pane("spawned.0", "scout");
    rig.start(&pane, "spawned.0", "claude");
    rig.kill_tools(&pane);
    let meta = rig.dir.join("meta");
    let text = std::fs::read_to_string(&meta).unwrap_or_default();
    // REPLACED, never appended. `_run` records rows of its own at a first
    // start, and ae reads a key that appears TWICE as no value at all — a
    // fixture that appended would silently hand the verb an empty conversation.
    // The working copy goes in the canonical spelling for the same reason the
    // carry needs it: the pane is respawned into it and `getcwd` answers with
    // it, so the copy's project key and the resume probe's cannot disagree.
    let mut text = text
        .lines()
        .filter(|line| {
            ![
                "work_dir=",
                "harness_session.spawned.0=",
                "capture_floor.spawned.0=",
                "launch_time.spawned.0=",
                "observed_model.spawned.0=",
                "config_home.spawned.0=",
                "config_home_base.spawned.0=",
            ]
            .iter()
            .any(|key| line.starts_with(key))
        })
        .fold(String::new(), |mut kept, line| {
            let _ = writeln!(kept, "{line}");
            kept
        });
    let _ = writeln!(
        text,
        "work_dir={}\nharness_session.spawned.0={ID}\ncapture_floor.spawned.0=100\n\
         launch_time.spawned.0=111\nobserved_model.spawned.0=Opus 5\n\
         config_home.spawned.0={}",
        work_dir(rig).display(),
        account(rig, "a").display()
    );
    assert!(std::fs::write(&meta, text).is_ok(), "the seat's history");
    plant_store(rig, &account(rig, "a"));
    pane
}

fn reseat(rig: &Rig, profile: &str) -> (Option<i32>, String, String) {
    rig.run_top(
        &rig.main_pane.clone(),
        &["reseat", &rig.session, "scout", "--using", profile],
    )
}

#[test]
fn a_move_between_two_accounts_of_one_tool_carries_the_whole_conversation() {
    let rig = Rig::new("carry");
    let pane = dead_seat(&rig);
    let (from, to) = (account(&rig, "a"), account(&rig, "b"));
    let key = ae::carry::project_key(&work_dir(&rig));

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(0), "out={out} err={err}");
    // RULING 1: the crossing is named exactly once, and it names both accounts.
    let crossings: Vec<&str> = out
        .lines()
        .filter(|line| line.contains("carried conversation"))
        .collect();
    assert_eq!(crossings.len(), 1, "one crossing line, got {out}");
    assert!(
        crossings[0].contains(ID)
            && crossings[0].contains(&from.display().to_string())
            && crossings[0].contains(&to.display().to_string()),
        "the line names the conversation and both accounts: {out}"
    );

    // THE FILE SET, one file of each kind ae measured.
    let project = to.join("projects").join(&key);
    assert_eq!(
        std::fs::read(project.join(format!("{ID}.jsonl"))).ok(),
        Some(TRANSCRIPT.to_vec()),
        "the transcript is in the target account, byte for byte"
    );
    for (what, path) in [
        ("tool results", project.join(ID).join("tool-results/t1.txt")),
        ("project memory", project.join("memory/notes.md")),
        ("checkpoints", to.join("file-history").join(ID).join("h@v1")),
        ("tasks", to.join("tasks").join(ID).join("1.json")),
    ] {
        assert!(path.exists(), "{what} did not travel: {}", path.display());
    }
    assert!(
        to.join("session-env").join(ID).is_dir(),
        "an empty sidecar directory is still part of the set"
    );
    // COPIED, NEVER MOVED: the source account is the rollback.
    assert_eq!(
        std::fs::read(from.join("projects").join(&key).join(format!("{ID}.jsonl"))).ok(),
        Some(TRANSCRIPT.to_vec()),
        "the source conversation is untouched"
    );

    // THE META: the seat moved, the conversation did NOT.
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
    assert_eq!(
        rig.meta_row("harness_session.spawned.0"),
        ID,
        "the conversation is kept, not replaced"
    );
    assert_eq!(
        rig.meta_row("capture_floor.spawned.0"),
        "100",
        "a retained exact conversation keeps the floor it was born under"
    );
    assert!(
        !rig.meta().contains("harness_session_prior.spawned.0="),
        "nothing was abandoned, so nothing becomes a predecessor:\n{}",
        rig.meta()
    );
    // The store rows name the TARGET, published in the same replacement.
    assert_eq!(
        rig.meta_row("config_home.spawned.0"),
        to.display().to_string(),
        "the account rows follow the conversation"
    );
    // Nothing for a human to re-send by hand: the conversation itself arrived.
    assert!(
        !rig.dir.join("seed.scout.md").exists(),
        "a carried seat is handed no seed pack"
    );
    assert!(
        out.contains("carried") && !out.contains("seed ok"),
        "the closing line says what happened instead of a seed verdict: {out}"
    );
    // THE AUDIT: one word, on the reseat record.
    assert!(
        rig.events().contains(", carried]"),
        "the record says the conversation was carried:\n{}",
        rig.events()
    );
    assert!(rig.tool_pid(&pane, "claude").is_some(), "the seat is up");
}

#[test]
fn the_carried_seat_resumes_its_conversation_instead_of_creating_a_second_one() {
    // THE BLOCKER THIS SLICE TURNS ON. `run::clear_slot` deletes
    // `launch.<slot>.started`, and `_run` reads exactly that file to choose
    // between creating a conversation and resuming one. A carry that did not
    // put it back would copy the entire store and then open a NEW conversation
    // beside it — every other pin above would still pass.
    let rig = Rig::new("resume");
    let pane = dead_seat(&rig);

    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert_eq!(code, Some(0), "out={out} err={err}");

    assert!(
        rig.dir.join("launch.spawned.0.started").exists(),
        "the slot is marked as already launched, or the next re-run creates again"
    );
    // THE WHOLE LOG, with no parsing: the fake APPENDS each launch, so two
    // launches racing to write one file is a fixture race and not a fact about
    // the product. Both claims hold over the text as a whole.
    let launched = rig.launched();
    assert!(
        launched.contains(&format!("--resume {ID}")),
        "the successor resumes the carried conversation — that id exists only in \
         the meta this move carried, so nothing else could have asked for it:\n{launched}"
    );
    assert!(
        !launched.contains("--session-id"),
        "no launch minted a conversation: a carried seat that fell back to CREATE \
         would be handed the fresh uuid the move would then have recorded:\n{launched}"
    );
    assert!(rig.tool_pid(&pane, "claude").is_some(), "the seat is up");
}

#[test]
fn a_target_account_already_holding_another_conversation_falls_back_loudly() {
    let rig = Rig::new("clash");
    dead_seat(&rig);
    let (from, to) = (account(&rig, "a"), account(&rig, "b"));
    let key = ae::carry::project_key(&work_dir(&rig));
    let clash = to.join("projects").join(&key).join(format!("{ID}.jsonl"));
    write(&clash, b"someone else's conversation\n");

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(0), "the seat still moves: out={out} err={err}");
    assert!(
        err.contains("could not carry") && err.contains(ID),
        "the fallback is LOUD and names the conversation: {err}"
    );
    assert_eq!(
        std::fs::read(&clash).ok(),
        Some(b"someone else's conversation\n".to_vec()),
        "nothing in the target was overwritten"
    );
    assert_eq!(
        std::fs::read(from.join("projects").join(&key).join(format!("{ID}.jsonl"))).ok(),
        Some(TRANSCRIPT.to_vec()),
        "and the source is untouched"
    );
    // TODAY'S PATH, in full: a fresh conversation, a predecessor, a seed pack.
    assert_ne!(rig.meta_row("harness_session.spawned.0"), ID);
    assert!(
        rig.meta_row("harness_session_prior.spawned.0").contains(ID),
        "the conversation ae could not carry is left addressable:\n{}",
        rig.meta()
    );
    assert!(rig.dir.join("seed.scout.md").exists(), "the seed is back");
    assert!(
        rig.events().contains(", seeded ("),
        "the record says the carry was tried and why it did not happen:\n{}",
        rig.events()
    );
}

#[test]
fn an_interrupted_carry_converges_instead_of_refusing_forever() {
    // The crash window: a re-run finds ITS OWN files in the target. They are
    // proven byte-identical, so nothing is written and nothing is touched —
    // the alternative would be a seat that can never be carried again.
    let rig = Rig::new("again");
    dead_seat(&rig);
    let to = account(&rig, "b");
    let key = ae::carry::project_key(&work_dir(&rig));
    let landed = to.join("projects").join(&key).join(format!("{ID}.jsonl"));
    write(&landed, TRANSCRIPT);
    let before = std::fs::metadata(&landed).and_then(|meta| meta.modified());

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(out.contains("carried"), "it converges on a carry: {out}");
    assert_eq!(rig.meta_row("harness_session.spawned.0"), ID);
    assert_eq!(
        std::fs::metadata(&landed)
            .and_then(|meta| meta.modified())
            .ok(),
        before.ok(),
        "an identical file is left exactly as it is, mtime included"
    );
}

#[test]
fn a_linked_conversation_file_is_refused_rather_than_followed() {
    let rig = Rig::new("link");
    dead_seat(&rig);
    let from = account(&rig, "a");
    let key = ae::carry::project_key(&work_dir(&rig));
    // A sidecar reached through a link could copy bytes out of any directory
    // on the machine into the other account.
    let linked = from.join("file-history").join(ID).join("h@v1");
    assert!(std::fs::remove_file(&linked).is_ok());
    assert!(
        std::os::unix::fs::symlink(rig.dir.join("meta"), &linked).is_ok(),
        "a link in the source store"
    );

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(0), "the seat still moves: out={out} err={err}");
    assert!(
        err.contains("could not carry") && err.contains("symbolic link"),
        "the link is named, not followed: {err}"
    );
    assert!(
        !account(&rig, "b")
            .join("projects")
            .join(&key)
            .join(format!("{ID}.jsonl"))
            .exists(),
        "the copy set is binding: no transcript lands when part of it cannot"
    );
    assert!(rig.dir.join("seed.scout.md").exists(), "today's path");
}

#[test]
fn a_project_memory_the_target_already_has_is_kept_and_said() {
    let rig = Rig::new("memory");
    dead_seat(&rig);
    let to = account(&rig, "b");
    let key = ae::carry::project_key(&work_dir(&rig));
    let theirs = to.join("projects").join(&key).join("memory").join("own.md");
    write(&theirs, b"the target account's own memory");

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(
        out.contains("memory") && out.contains("kept"),
        "the human is told which memory the successor reads: {out}"
    );
    assert_eq!(
        std::fs::read(&theirs).ok(),
        Some(b"the target account's own memory".to_vec()),
        "it is untouched"
    );
    assert!(
        !theirs.with_file_name("notes.md").exists(),
        "and never merged with the one that was left behind"
    );
}

#[test]
fn a_conversation_ae_cannot_name_takes_todays_path_without_a_word() {
    // `pending` is the word for a conversation that never resolved. There is
    // nothing to copy, and the move must be BYTE-IDENTICAL to what it was
    // before carrying existed — no crossing line, no carry word in the record.
    let rig = Rig::new("pending");
    dead_seat(&rig);
    let meta = rig.dir.join("meta");
    let text = std::fs::read_to_string(&meta).unwrap_or_default().replace(
        &format!("harness_session.spawned.0={ID}"),
        "harness_session.spawned.0=pending",
    );
    assert!(std::fs::write(&meta, text).is_ok());

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(!out.contains("carried"), "nothing is claimed: {out}");
    assert!(
        !err.contains("could not carry"),
        "and nothing is blamed: {err}"
    );
    assert!(rig.dir.join("seed.scout.md").exists(), "today's path");
    let events = rig.events();
    assert!(
        !events.contains(", carried]") && !events.contains(", seeded ("),
        "a move that was never a carry question records what it always did:\n{events}"
    );
}

#[test]
fn a_move_to_another_tool_is_never_a_carry() {
    // The existing `tests/it/reseat.rs` pins prove the tool-change path whole;
    // this one pins the BOUNDARY: the same account rows and a good id, and the
    // carry still does not fire, because the successor cannot read those files.
    let rig = Rig::new("other");
    dead_seat(&rig);

    let (code, out, err) = reseat(&rig, "fake-opencode");

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(
        !out.contains("carried"),
        "a tool change carries nothing: {out}"
    );
    assert_eq!(rig.meta_row("harness_session.spawned.0"), "pending");
    assert!(
        rig.meta_row("harness_session_prior.spawned.0").contains(ID),
        "the conversation it left is a predecessor, as it always was"
    );
}
