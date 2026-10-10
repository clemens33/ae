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
    planted_seat(rig, Liveness::Dead)
}

/// The same seat with its tool STILL RUNNING, so the move has to stop it first
/// and writes the stop's own record on the way. That record is the one a carry
/// must not let speak about the conversation.
fn running_seat(rig: &Rig) -> String {
    planted_seat(rig, Liveness::Running)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Liveness {
    Dead,
    Running,
}

fn planted_seat(rig: &Rig, liveness: Liveness) -> String {
    rig.seat_rows("spawned.0", "scout", "claude-a", "claude");
    let pane = rig.new_pane("spawned.0", "scout");
    rig.start(&pane, "spawned.0", "claude");
    if liveness == Liveness::Dead {
        rig.kill_tools(&pane);
    }
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
fn a_running_seats_stop_record_says_nothing_about_the_conversation_it_carries() {
    // THE OTHER RECORD. A seat whose tool is still running is STOPPED first,
    // and that stop is audited the moment the pane is back at its shell —
    // BEFORE the carry verdict exists. It therefore cannot name the
    // conversation truthfully, so it names none at all: `prior` there would
    // call a conversation abandoned while ae was in the middle of carrying it.
    let rig = Rig::new("carrystop");
    let pane = running_seat(&rig);

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(
        code,
        Some(0),
        "out={out} err={err}\nframe was:\n{}",
        rig.capture(&pane)
    );
    assert!(out.contains("carried conversation"), "it carried: {out}");
    let events = rig.events();
    // THE STOP HAPPENED, and is on the record — this pin is worth nothing if
    // the seat was never running in the first place.
    let stop = events
        .lines()
        .find(|line| line.contains("stopped claude in place"))
        .unwrap_or_else(|| panic!("the running tool was stopped and audited:\n{events}"));
    assert!(
        !stop.contains(ID),
        "the stop record names no conversation at all: {stop}"
    );
    // NOWHERE, on any record: the conversation this move carried is live.
    assert!(
        !events.contains(&format!("prior {ID}")),
        "nothing calls the carried conversation prior:\n{events}"
    );
    // And the MOVE record still says what became of it.
    assert!(
        events.contains(&format!("conversation {ID}, carried]")),
        "the move record names the conversation it carried:\n{events}"
    );
}

#[test]
fn a_move_between_two_accounts_of_one_tool_carries_the_whole_conversation() {
    let rig = Rig::new("carry");
    let pane = dead_seat(&rig);
    let (from, to) = (account(&rig, "a"), account(&rig, "b"));
    let key = ae::carry::project_key(&work_dir(&rig));

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(!err.contains("starts a fresh"), "no note: {err}");
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
    // THE AUDIT: one word, on the reseat record — and the id filed under what
    // it IS. A carried conversation continues in the other account, so a record
    // calling it `prior` would send a later reader after a live conversation
    // believing it dead.
    assert!(
        rig.events()
            .contains(&format!("conversation {ID}, carried]")),
        "the record names the conversation it carried, not one left behind:\n{}",
        rig.events()
    );
    assert!(
        !rig.events().contains(&format!("prior {ID}")),
        "nothing calls the carried conversation prior:\n{}",
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
fn a_target_account_already_holding_another_conversation_is_refused_before_the_move() {
    let rig = Rig::new("clash");
    dead_seat(&rig);
    let (from, to) = (account(&rig, "a"), account(&rig, "b"));
    let key = ae::carry::project_key(&work_dir(&rig));
    let clash = to.join("projects").join(&key).join(format!("{ID}.jsonl"));
    write(&clash, b"someone else's conversation\n");
    let meta = rig.meta();

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(1), "refused first: out={out} err={err}");
    assert!(
        err.contains("could not carry") && err.contains(ID),
        "the refusal is LOUD and names the conversation: {err}"
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
    assert_eq!(rig.meta(), meta, "the seat is exactly what it was");
    assert!(!rig.dir.join("seed.scout.md").exists(), "no seed: no move");
}

#[test]
fn a_running_seat_whose_carry_would_fail_is_refused_before_it_is_stopped() {
    // THE PRE-STOP CHECK reads and compares the whole set under the lock BEFORE
    // the running tool is ended, so a target holding a different sidecar costs
    // a refusal — not a stopped tool and a fresh conversation.
    let rig = Rig::new("prestop");
    let pane = running_seat(&rig);
    let theirs = account(&rig, "b")
        .join("file-history")
        .join(ID)
        .join("h@v1");
    write(&theirs, b"another checkpoint");
    let pid = rig.tool_pid(&pane, "claude");
    let (meta, events) = (rig.meta(), rig.events());

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("could not carry") && err.contains("nothing was stopped"),
        "{err}"
    );
    assert!(
        pid.is_some() && rig.tool_pid(&pane, "claude") == pid,
        "never stopped"
    );
    assert_eq!(
        (rig.meta(), rig.events()),
        (meta, events),
        "nothing written"
    );
    assert_eq!(
        std::fs::read(&theirs).ok(),
        Some(b"another checkpoint".to_vec())
    );
}

#[test]
fn a_carry_that_fails_only_once_it_writes_falls_back_loudly() {
    // The pre-stop Check writes nothing, so a target ae may not WRITE into
    // passes it and fails only the copy, past the stop: the seeded fallback.
    use std::os::unix::fs::PermissionsExt as _;

    let rig = Rig::new("rofall");
    dead_seat(&rig);
    let to = account(&rig, "b");
    assert!(std::fs::create_dir_all(&to).is_ok(), "the target account");
    assert!(std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o500)).is_ok());

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    let _ = std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o700));
    assert_eq!(code, Some(0), "the seat still moves: out={out} err={err}");
    assert!(
        err.contains("could not carry") && err.contains(ID),
        "the fallback is LOUD and names the conversation: {err}"
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
        rig.events().contains(&format!("prior {ID}, seeded (")),
        "the record says the carry was tried, why it did not happen, and that \
         this conversation really was left behind:\n{}",
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

    assert_eq!(code, Some(1), "refused first: out={out} err={err}");
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
    assert!(!rig.dir.join("seed.scout.md").exists(), "no seed: no move");
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

#[test]
fn a_store_reached_through_a_linked_ancestor_is_refused_rather_than_followed() {
    // The LEAF lstat is not the whole rule: a read or a listing re-opens the
    // entire pathname, so a sidecar ROOT that is a link would file another
    // directory's bytes under this conversation's name in the other account.
    let rig = Rig::new("ances");
    dead_seat(&rig);
    let (from, to) = (account(&rig, "a"), account(&rig, "b"));
    let key = ae::carry::project_key(&work_dir(&rig));
    let decoy = canonical_scratch(&rig).join("decoy");
    write(&decoy.join(ID).join("h@v1"), b"elsewhere");
    let root = from.join("file-history");
    assert!(std::fs::remove_dir_all(&root).is_ok(), "the real root goes");
    assert!(
        std::os::unix::fs::symlink(&decoy, &root).is_ok(),
        "and a link takes its place"
    );

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(1), "refused first: out={out} err={err}");
    assert!(
        err.contains("could not carry") && err.contains("is a symbolic link"),
        "the refusal is loud and says what it refused: {err}"
    );
    assert!(
        !to.join("file-history").join(ID).exists(),
        "nothing was read through the link"
    );
    assert!(
        !to.join("projects")
            .join(&key)
            .join(format!("{ID}.jsonl"))
            .exists(),
        "and the conversation never became findable"
    );
    assert!(!rig.dir.join("seed.scout.md").exists(), "no seed: no move");
}

#[test]
fn a_sidecar_ae_cannot_even_stat_is_never_silently_left_behind() {
    // The copy set is BINDING. An `lstat` that fails for any reason but absence
    // is a node ae cannot promise it copied, so it abandons the carry instead
    // of reporting one that is quietly short. (Run as root this fails loudly
    // rather than passing: the mode below stops being a refusal.)
    use std::os::unix::fs::PermissionsExt as _;

    let rig = Rig::new("eacces");
    dead_seat(&rig);
    let to = account(&rig, "b");
    let key = ae::carry::project_key(&work_dir(&rig));
    let closed = account(&rig, "a").join("tasks");
    assert!(
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000)).is_ok(),
        "the sidecar root closes"
    );

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    let _ = std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o700));
    assert_eq!(code, Some(1), "refused first: out={out} err={err}");
    assert!(
        err.contains("could not carry"),
        "the refusal is loud: {err}"
    );
    assert!(
        !to.join("tasks").join(ID).exists()
            && !to
                .join("projects")
                .join(&key)
                .join(format!("{ID}.jsonl"))
                .exists(),
        "a carry that cannot read its whole set delivers none of it"
    );
    assert!(!rig.dir.join("seed.scout.md").exists(), "no seed: no move");
}

#[test]
fn a_target_project_memory_that_is_not_a_directory_is_never_called_kept() {
    // Only a real directory is the target account's own memory. Anything else
    // there is a target ae cannot explain — and calling it "kept" would tell a
    // human their successor is reading a memory that does not exist.
    let rig = Rig::new("memfil");
    dead_seat(&rig);
    let to = account(&rig, "b");
    let key = ae::carry::project_key(&work_dir(&rig));
    let memory = to.join("projects").join(&key).join("memory");
    write(&memory, b"not a directory\n");

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(code, Some(1), "refused first: out={out} err={err}");
    assert!(
        err.contains("could not carry") && err.contains("memory"),
        "the refusal names what it found: {err}"
    );
    assert!(
        !out.contains("kept, never merged"),
        "a file is not another account's memory: {out}"
    );
    assert_eq!(
        std::fs::read(&memory).ok(),
        Some(b"not a directory\n".to_vec()),
        "and it was not written through"
    );
    assert!(!rig.dir.join("seed.scout.md").exists(), "no seed: no move");
}

#[test]
fn a_seat_whose_resume_marker_cannot_be_written_moves_nothing_at_all() {
    // `clear_slot` removes the marker `_run` reads to choose resume over
    // create, and a carried seat has to have it back. The two files cannot be
    // one write, so the failure is taken BEFORE anything moves: the alternative
    // is a copied store beside a successor that starts a new conversation.
    let rig = Rig::new("marker");
    dead_seat(&rig);
    let to = account(&rig, "b");
    let key = ae::carry::project_key(&work_dir(&rig));
    let marker = rig.dir.join("launch.spawned.0.started");
    assert!(
        std::fs::remove_file(&marker).is_ok(),
        "the seat was started, so the marker was there"
    );
    assert!(
        std::fs::create_dir(&marker).is_ok(),
        "and a directory takes its name"
    );

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_ne!(code, Some(0), "the move refuses: out={out} err={err}");
    assert!(
        !to.join("projects")
            .join(&key)
            .join(format!("{ID}.jsonl"))
            .exists(),
        "and nothing was copied: the refusal comes before the move"
    );
    assert_eq!(
        rig.meta_row("harness_session.spawned.0"),
        ID,
        "the seat is exactly what it was"
    );
    assert_eq!(
        rig.meta_row("config_home.spawned.0"),
        account(&rig, "a").display().to_string(),
        "on the account it was on"
    );
}

#[test]
fn a_clear_that_fails_after_the_marker_leaves_the_seat_able_to_resume() {
    // `clear_slot` removes the start marker FIRST and can fail on a later file.
    // The meta has not moved, so the seat is still its old self — and its old
    // self had that marker. Without it the next start would open a NEW
    // conversation on the account the seat still records.
    let rig = Rig::new("clrfail");
    dead_seat(&rig);
    let marker = rig.dir.join("launch.spawned.0.started");
    let prompt = rig.dir.join("launch.spawned.0.prompt");
    let _ = std::fs::remove_file(&prompt);
    assert!(
        std::fs::create_dir(&prompt).is_ok(),
        "a launch file `clear_slot` cannot remove, AFTER the marker in its order"
    );

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_ne!(code, Some(0), "the move refuses: out={out} err={err}");
    assert!(
        err.contains("could not be cleared") && !err.contains("could not be put back"),
        "it says what failed, and the marker was not one of them: {err}"
    );
    assert!(
        marker.is_file(),
        "the seat can still resume the conversation it records"
    );
    assert_eq!(
        rig.meta_row("harness_session.spawned.0"),
        ID,
        "and nothing about the seat moved"
    );
}
// Frozen NAV acceptance. Append byte-identically to tests/it/reseat_carry.rs.
// Existing helpers run a real private tmux server with external harness fakes.
const AUTO_KEY: &str = "2026-09-25T10:00:00Z";

fn automatic_carry(rig: &Rig, to: &str) -> String {
    use std::io::Write as _;
    let pane = running_seat(rig);
    rig.mark_limited(&pane);
    let config = rig.scratch.join("config");
    let text = std::fs::read_to_string(&config).unwrap_or_else(|why| panic!("config: {why}"));
    std::fs::write(
        config,
        format!("{text}auto_reseat = on\n[auto_reseat]\nfake-claude-a = {to}\n"),
    )
    .unwrap_or_else(|why| panic!("candidate: {why}"));
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), rig.dir.join("send"))
        .unwrap_or_else(|why| panic!("helper: {why}"));
    for (action, reference) in [("limit", ""), (ae::autoreseat::ATTEMPT_ACTION, AUTO_KEY)] {
        let line = format!(
            "{{\"ts\":\"{AUTO_KEY}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session\":\"{}\",\"ref\":\"{reference}\"}}\n",
            rig.session
        );
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(rig.dir.join("events.jsonl"))
            .unwrap_or_else(|why| panic!("journal: {why}"))
            .write_all(line.as_bytes())
            .unwrap_or_else(|why| panic!("episode: {why}"));
    }
    pane
}

fn automatic_leg(rig: &Rig) -> (Option<i32>, String) {
    let out = super::cli::ae()
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env("AE_HOME", &rig.scratch)
        .env("HOME", rig.scratch.join("home"))
        .env("AE_SEND_DEFER_SEC", "0")
        .env_remove("AE_SENDER_OVERRIDE")
        .arg("_auto-reseat")
        .arg(&rig.dir)
        .args(["spawned.0", AUTO_KEY])
        .output()
        .unwrap_or_else(|why| panic!("leg: {why}"));
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn automatic_carried_reseat_gets_one_authorized_continuation_and_keeps_its_conversation() {
    use std::io::Write as _;
    let rig = Rig::new("autokick");
    let pane = automatic_carry(&rig, "fake-claude-b");
    let pending = "PENDING-BODY-MUST-NOT-BE-RESENT";
    write(
        &rig.dir.join("memo.tsv"),
        b"2026-09-25T10:00:00Z\tscout\tparking\tparked scope stays parked\n",
    );
    let memo_before = std::fs::read(rig.dir.join("memo.tsv")).expect("memo");
    let mut log = std::fs::OpenOptions::new()
        .append(true)
        .open(rig.dir.join("events.jsonl"))
        .expect("journal");
    writeln!(log, "{{\"ts\":\"{AUTO_KEY}\",\"actor\":\"scout\",\"actor_slot\":\"spawned.0\",\"actor_session\":\"{}\",\"action\":\"ask\",\"target\":\"lead\",\"target_slot\":\"main\",\"target_session\":\"{}\",\"ref\":\"ae-20260925T100000Z-00000001\",\"summary\":\"{pending}\"}}", rig.session, rig.session).expect("pending request");
    let (code, err) = automatic_leg(&rig);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        rig.tool_pid(&pane, "claude").is_some(),
        "replacement observed"
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
    assert_eq!(rig.meta_row("harness_session.spawned.0"), ID);
    let transcript = account(&rig, "b")
        .join("projects")
        .join(ae::carry::project_key(&work_dir(&rig)))
        .join(format!("{ID}.jsonl"));
    assert_eq!(
        std::fs::read(transcript).expect("carried bytes"),
        TRANSCRIPT
    );
    assert!(
        rig.launched().contains(&format!("--resume {ID}")),
        "{}",
        rig.launched()
    );
    let received = rig.received();
    assert_eq!(
        received.matches("⟦ae:").count(),
        1,
        "one continuation: {received}"
    );
    let words = received.to_lowercase();
    for part in [
        "reseat",
        "auto",
        "state",
        "pending",
        "request",
        "memo",
        "brief",
        "authorized",
        "wait",
        "park",
    ] {
        assert!(
            words.contains(part),
            "continuation missing {part}: {received}"
        );
    }
    assert!(
        !received.contains(pending),
        "never replay pending request bodies"
    );
    assert!(
        !received.contains("seed pack"),
        "carried history needs no second seed"
    );
    assert_eq!(
        std::fs::read(rig.dir.join("memo.tsv")).expect("memo"),
        memo_before
    );
    let events = rig.events();
    assert_eq!(
        events.matches(pending).count(),
        1,
        "pending ledger preserved"
    );
    assert!(!events.contains("\"action\":\"reply\""), "no forged answer");
    let launches = rig.launched();
    let _ = automatic_leg(&rig);
    assert_eq!(rig.received(), received, "closed episode cannot kick twice");
    assert_eq!(rig.launched(), launches, "no second move");
}

#[test]
fn fresh_automatic_and_manual_carried_reseats_do_not_get_a_second_continuation() {
    let manual = Rig::new("manualnokick");
    dead_seat(&manual);
    let (code, out, err) = reseat(&manual, "fake-claude-b");
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(
        manual.received().is_empty(),
        "manual carried reseat unchanged"
    );
    let fresh = Rig::new("freshnokick");
    automatic_carry(&fresh, "fake-opencode");
    let (code, err) = automatic_leg(&fresh);
    assert_eq!(code, Some(0), "{err}");
    let received = fresh.received();
    assert_eq!(
        received.matches("⟦ae:").count(),
        1,
        "one seed only: {received}"
    );
    assert_eq!(received.matches("## 10. successor instructions").count(), 1);
}

#[test]
fn a_blocked_auto_continuation_leaves_the_move_intact_and_reports_an_explicit_next_step() {
    for (tag, draft) in [("kickdraft", true), ("kickunreadable", false)] {
        let rig = Rig::new(tag);
        let pane = automatic_carry(&rig, "fake-claude-b");
        // Only the resumed external fake opens a draft/unreadable composer. The old
        // seat remains eligible for reseat; delivery must respect the new one.
        let script = rig.scratch.join("claude.pl");
        let fake = std::fs::read_to_string(&script).expect("fake");
        let changed = if draft {
            let on_resume = format!(
                "use strict;\nif (grep {{ $_ eq '--resume' }} @ARGV) {{ open(my $f, '>', '{}') or die; close($f); }}",
                rig.scratch.join("__DRAFT__").display()
            );
            fake.replacen("use strict;", &on_resume, 1)
        } else {
            assert!(fake.contains("my $frame = 1;"));
            fake.replacen(
                "my $frame = 1;",
                "my $frame = (grep { $_ eq '--resume' } @ARGV) ? 0 : 1;",
                1,
            )
        };
        std::fs::write(script, changed).expect("resumed fake");
        let (_, err) = automatic_leg(&rig);
        assert_eq!(
            rig.meta_row("profile.spawned.0"),
            "fake-claude-b",
            "move intact: {err}"
        );
        assert_eq!(rig.meta_row("harness_session.spawned.0"), ID);
        assert!(
            rig.tool_pid(&pane, "claude").is_some(),
            "replacement stays up"
        );
        assert!(
            rig.received().is_empty(),
            "draft/unreadable gets no paste: {}",
            rig.received()
        );
        let events = rig.events();
        let chat: Vec<_> = events
            .lines()
            .filter_map(|line| ae::events::Event::parse_line(line).ok())
            .filter(|event| event.action == "chat")
            .collect();
        assert_eq!(chat.len(), 1, "one truthful notice: {events}");
        let notice = chat[0]
            .summary
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();
        assert!(
            notice.contains("moved") && notice.contains("nudge"),
            "{notice}"
        );
        assert!(
            notice.contains("unconfirmed")
                || notice.contains("refused")
                || notice.contains("undelivered"),
            "{notice}"
        );
        assert!(
            notice.contains("next:") && !notice.contains("next: nothing"),
            "explicit recovery: {notice}"
        );
        assert!(
            !notice.contains("move failed")
                && !notice.contains("relaunch")
                && !notice.contains("reseat it"),
            "never suggest another move: {notice}"
        );
        let launches = rig.launched();
        let _ = automatic_leg(&rig);
        assert_eq!(rig.launched(), launches, "no automatic retry move");
        assert_eq!(
            rig.events(),
            events,
            "failure already observable; no duplicate retry"
        );
    }
}
#[test]
fn a_failed_continuation_keeps_status_and_safe_recovery_with_maximum_agent_names() {
    use ae::autoreseat::{Ending, Move, NOTICE_CHARS, Nudge, notice};
    let agent = "n".repeat(64);
    assert!(ae::config::is_agent_name(&agent));
    let profile = "p".repeat(64);
    let model = "m".repeat(100);
    for (nudge, status, guard) in [
        (
            Nudge::Unconfirmed,
            "unconfirmed",
            "only if the nudge still sits in its input box",
        ),
        (
            Nudge::Refused,
            "refused or unrecorded",
            "only if no nudge reached its pane",
        ),
    ] {
        let moved = Move {
            from: &profile,
            to: &profile,
            tool: ("claude", "claude"),
            model: (&model, &model),
            carried: Some(nudge),
            critical: true,
            dirty: true,
            cause: "",
        };
        let shown = notice("aedev", &agent, &Ending::Moved(moved));
        assert!(shown.chars().count() <= NOTICE_CHARS);
        assert!(
            shown.contains("continuation nudge") && shown.contains(status),
            "{shown}"
        );
        assert!(shown.contains("Next:") && shown.contains("peek"), "{shown}");
        assert!(
            shown.contains(guard),
            "safe resend condition clipped: {shown}"
        );
    }
}
