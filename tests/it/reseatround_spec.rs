//! Frozen acceptance for account round trips, from brief R1–R6.
//!
//! Real ae, real private tmux, real parsed account profiles. The external
//! harness is the existing interpreter fake; it does not implement carry.
//! Account bytes are synthetic and opaque. Archive layout is not pinned to
//! a root name or timestamp spelling: its relative file names, containment,
//! exclusive move, permissions and human/audit receipts are the contract.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance helpers inspect private account homes and fail loudly on invalid fixtures"
)]

use std::fmt::Write as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::seat_relaunch::Rig;

const ID: &str = "22222222-2222-4222-8222-222222222222";
const TRANSCRIPT: &[u8] = b"user\nassistant\nmore\x00\xff\n";
const STALE: &[u8] = b"user\n";

fn root(rig: &Rig) -> PathBuf {
    std::fs::canonicalize(&rig.scratch).expect("fixture scratch canonicalizes")
}

fn account(rig: &Rig, name: &str) -> PathBuf {
    root(rig).join(format!("home-{name}"))
}

fn relative_transcript(rig: &Rig) -> PathBuf {
    PathBuf::from("projects")
        .join(ae::carry::project_key(&root(rig).join("work")))
        .join(format!("{ID}.jsonl"))
}

fn sidecars(rig: &Rig) -> [PathBuf; 4] {
    [
        relative_transcript(rig)
            .parent()
            .expect("project directory")
            .join(ID)
            .join("tool-results/t1.txt"),
        PathBuf::from("file-history").join(ID).join("h@v1"),
        PathBuf::from("tasks").join(ID).join("1.json"),
        PathBuf::from("session-env").join(ID).join("env.txt"),
    ]
}

fn put(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("fixture file parent")).expect("fixture parents");
    std::fs::write(path, bytes).expect("fixture bytes");
}

fn old_mtime(path: &Path) {
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(123_456_789);
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("fixture writable file")
        .set_times(std::fs::FileTimes::new().set_modified(time))
        .expect("fixture old mtime");
}

fn plant(rig: &Rig, running: bool) -> String {
    plant_with_command(rig, running, "/bin/sh")
}

fn plant_with_command(rig: &Rig, running: bool, command: &str) -> String {
    rig.seat_rows("spawned.0", "scout", "claude-a", "claude");
    let pane = rig.new_pane_running("spawned.0", "scout", command);
    rig.start(&pane, "spawned.0", "claude");
    if !running {
        rig.kill_tools(&pane);
    }
    let rows = rig.meta();
    let mut rows = rows
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
    writeln!(
        rows,
        "work_dir={}\nharness_session.spawned.0={ID}\ncapture_floor.spawned.0=100\n\
         launch_time.spawned.0=111\nobserved_model.spawned.0=Opus 5\n\
         config_home.spawned.0={}",
        root(rig).join("work").display(),
        account(rig, "a").display()
    )
    .expect("fixture rows");
    std::fs::write(rig.dir.join("meta"), rows).expect("fixture seat history");
    put(
        &account(rig, "a").join(relative_transcript(rig)),
        TRANSCRIPT,
    );
    for relative in sidecars(rig) {
        put(&account(rig, "a").join(relative), b"source sidecar");
    }
    std::fs::create_dir_all(account(rig, "b")).expect("fixture target account");
    pane
}

fn reseat(rig: &Rig, profile: &str) -> (Option<i32>, String, String) {
    rig.run_top(
        &rig.main_pane,
        &["reseat", &rig.session, "scout", "--using", profile],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    relative: PathBuf,
    kind: &'static str,
    bytes: Vec<u8>,
    link: Option<PathBuf>,
    dev: u64,
    ino: u64,
    mode: u32,
    modified: SystemTime,
}

fn snapshot(home: &Path) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut pending = vec![home.to_path_buf()];
    while let Some(path) = pending.pop() {
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(why) if why.kind() == std::io::ErrorKind::NotFound && path == home => continue,
            Err(why) => panic!("fixture snapshot {}: {why}", path.display()),
        };
        let kind = meta.file_type();
        if kind.is_dir() {
            for entry in std::fs::read_dir(&path).expect("fixture directory readable") {
                pending.push(entry.expect("fixture directory entry").path());
            }
        }
        nodes.push(Node {
            relative: path.strip_prefix(home).expect("node inside fixture").into(),
            kind: if kind.is_symlink() {
                "link"
            } else if kind.is_dir() {
                "dir"
            } else {
                "file"
            },
            bytes: if kind.is_file() {
                std::fs::read(&path).expect("fixture regular file readable")
            } else {
                Vec::new()
            },
            link: kind
                .is_symlink()
                .then(|| std::fs::read_link(&path).expect("fixture link")),
            dev: meta.dev(),
            ino: meta.ino(),
            mode: meta.permissions().mode() & 0o7777,
            modified: meta.modified().expect("fixture mtime"),
        });
    }
    nodes.sort_by(|a, b| a.relative.cmp(&b.relative));
    nodes
}

fn file_at(home: &Path, relative: &Path) -> Node {
    snapshot(home)
        .into_iter()
        .find(|node| node.relative == relative)
        .expect("fixture file exists")
}

fn assert_resumed(rig: &Rig, pane: &str, home: &Path, bytes: &[u8]) {
    assert_eq!(
        rig.meta_row("harness_session.spawned.0"),
        ID,
        "R1 conversation retained"
    );
    assert_eq!(
        rig.meta_row("capture_floor.spawned.0"),
        "100",
        "R6 resume floor retained"
    );
    assert_eq!(
        rig.meta_row("config_home.spawned.0"),
        home.display().to_string()
    );
    assert_eq!(
        std::fs::read(home.join(relative_transcript(rig))).expect("carried transcript"),
        bytes
    );
    assert!(
        rig.launched().contains(&format!("--resume {ID}")),
        "R1 successor resumes same id"
    );
    assert!(
        !rig.dir.join("seed.scout.md").exists(),
        "R6 carried move needs no seed"
    );
    assert!(
        rig.tool_pid(pane, "claude").is_some(),
        "successor observed running"
    );
}

fn assert_archive(
    home: &Path,
    before: &[Node],
    archived: &[Node],
    out: &str,
    events: &str,
) -> PathBuf {
    let after = snapshot(home);
    let first = &archived[0];
    let moved = after
        .iter()
        .find(|node| {
            node.relative != first.relative && node.dev == first.dev && node.ino == first.ino
        })
        .expect("R3 old target inode is renamed aside, never deleted");
    let mut archive = home.join(&moved.relative);
    for _ in first.relative.components() {
        assert!(archive.pop(), "archive retains account-relative names");
    }
    assert!(
        archive.starts_with(home) && archive != home,
        "R3 archive stays inside target account"
    );
    for old in archived {
        let held = file_at(
            home,
            &archive
                .strip_prefix(home)
                .expect("archive within account")
                .join(&old.relative),
        );
        assert_eq!(
            (held.dev, held.ino),
            (old.dev, old.ino),
            "R3 rename preserves inode: {}",
            old.relative.display()
        );
        assert_eq!(held.bytes, old.bytes, "R3 archive preserves old bytes");
        assert_eq!(held.modified, old.modified, "R3 archive preserves mtime");
    }
    for node in &after {
        if node.kind == "dir" && !before.iter().any(|old| old.relative == node.relative) {
            assert_eq!(
                node.mode,
                0o700,
                "R3 new directories private: {}",
                node.relative.display()
            );
        }
    }
    for old in before.iter().filter(|node| node.kind == "file") {
        assert!(
            after
                .iter()
                .any(|node| node.kind == "file" && node.bytes == old.bytes),
            "R3 every pre-existing target byte remains findable: {}",
            old.relative.display()
        );
    }
    let named = archive.display().to_string();
    let lines: Vec<_> = out.lines().filter(|line| line.contains(&named)).collect();
    assert_eq!(
        lines.len(),
        1,
        "R5 one stdout line names archive directory: {out}"
    );
    let count = archived.len().to_string();
    assert!(
        lines[0].to_lowercase().contains("archiv")
            && lines[0]
                .split_whitespace()
                .any(|word| word.trim_matches(|ch: char| !ch.is_ascii_digit()) == count),
        "R5 stdout names archive count {}: {out}",
        archived.len()
    );
    let move_records: Vec<_> = events
        .lines()
        .filter_map(|line| ae::events::Event::parse_line(line).ok())
        .filter(|event| event.action == "reseat")
        .filter(|event| {
            event
                .summary
                .as_deref()
                .is_some_and(|summary| summary.starts_with("reseated "))
        })
        .collect();
    assert!(
        move_records.iter().any(|event| event
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains(&named))),
        "R5 move record carries archive directory: {events}"
    );
    archive
}

#[test]
fn stale_prefix_is_archived_then_carried_with_private_preserved_archive_and_receipt() {
    let rig = Rig::new("rrprefix");
    let pane = plant(&rig, false);
    let to = account(&rig, "b");
    let relative = relative_transcript(&rig);
    put(&to.join(&relative), STALE);
    old_mtime(&to.join(&relative));
    put(&to.join("unrelated/keep.bin"), b"target-only bytes");
    let before = snapshot(&to);
    let old = file_at(&to, &relative);
    let source = snapshot(&account(&rig, "a"));

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(
        code,
        Some(0),
        "R1 strict byte prefix must carry: out={out} err={err}"
    );
    assert_resumed(&rig, &pane, &to, TRANSCRIPT);
    assert_archive(&to, &before, &[old], &out, &rig.events());
    assert_eq!(
        snapshot(&account(&rig, "a")),
        source,
        "R6 source account untouched"
    );
}

#[test]
fn empty_target_transcript_is_also_a_strict_prefix() {
    let rig = Rig::new("rrempty");
    let pane = plant(&rig, false);
    let to = account(&rig, "b");
    put(&to.join(relative_transcript(&rig)), b"");
    let before = snapshot(&to);
    let old = file_at(&to, &relative_transcript(&rig));
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert_eq!(
        code,
        Some(0),
        "R1 zero-length strict prefix must carry: out={out} err={err}"
    );
    assert_resumed(&rig, &pane, &to, TRANSCRIPT);
    assert_archive(&to, &before, &[old], &out, &rig.events());
}

#[test]
fn differing_files_in_each_sidecar_tree_and_uuid_project_directory_are_archived() {
    let rig = Rig::new("rrsidecars");
    let pane = plant(&rig, true);
    let to = account(&rig, "b");
    put(&to.join(relative_transcript(&rig)), TRANSCRIPT);
    for (index, relative) in sidecars(&rig).iter().enumerate() {
        put(
            &to.join(relative),
            format!("old target sidecar {index}").as_bytes(),
        );
        old_mtime(&to.join(relative));
    }
    let before = snapshot(&to);
    let archived: Vec<_> = sidecars(&rig)
        .iter()
        .map(|relative| file_at(&to, relative))
        .collect();
    let old_transcript = file_at(&to, &relative_transcript(&rig));

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(
        code,
        Some(0),
        "R2 differing regular sidecars must carry: out={out} err={err}"
    );
    assert_resumed(&rig, &pane, &to, TRANSCRIPT);
    for relative in sidecars(&rig) {
        assert_eq!(
            std::fs::read(to.join(relative)).expect("replaced sidecar"),
            b"source sidecar"
        );
    }
    assert_eq!(
        file_at(&to, &relative_transcript(&rig)),
        old_transcript,
        "R1 equal transcript untouched"
    );
    assert_archive(&to, &before, &archived, &out, &rig.events());
}

#[test]
fn successive_round_trips_keep_conversation_and_allocate_fresh_archives() {
    let rig = Rig::new("rrtrip");
    let pane = plant(&rig, false);
    let (a, b) = (account(&rig, "a"), account(&rig, "b"));
    let relative = relative_transcript(&rig);
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert_eq!(code, Some(0), "fixture initial clean carry: {out} {err}");
    rig.kill_tools(&pane);
    let advanced = [TRANSCRIPT, b"next turn\n"].concat();
    put(&b.join(&relative), &advanced);
    put(&b.join(&sidecars(&rig)[0]), b"new b tool result");
    let before_a = snapshot(&a);
    let old_a = [file_at(&a, &relative), file_at(&a, &sidecars(&rig)[0])];
    let (code, out, err) = reseat(&rig, "fake-claude-a");
    assert_eq!(
        code,
        Some(0),
        "R1/R2 returning to older account must carry: {out} {err}"
    );
    assert_resumed(&rig, &pane, &a, &advanced);
    let first = assert_archive(&a, &before_a, &old_a, &out, &rig.events());
    rig.kill_tools(&pane);
    let later = [advanced.as_slice(), b"another turn\n"].concat();
    put(&a.join(&relative), &later);
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert_eq!(code, Some(0), "R1 second account return: {out} {err}");
    rig.kill_tools(&pane);
    let latest = [later.as_slice(), b"latest turn\n"].concat();
    put(&b.join(&relative), &latest);
    let before_a = snapshot(&a);
    let old_a = file_at(&a, &relative);
    let (code, out, err) = reseat(&rig, "fake-claude-a");
    assert_eq!(code, Some(0), "R3 repeated archive succeeds: {out} {err}");
    let second = assert_archive(&a, &before_a, &[old_a], &out, &rig.events());
    assert_ne!(
        first, second,
        "R3 fresh per-move archive never overwrites previous one"
    );
    assert_eq!(
        std::fs::read(first.join(&relative)).expect("first archive remains"),
        TRANSCRIPT
    );
    assert_resumed(&rig, &pane, &a, &latest);
}

fn assert_refuses_untouched(rig: &Rig, pane: &str) -> String {
    let from = account(rig, "a");
    let to = account(rig, "b");
    let (meta, events, launches) = (rig.meta(), rig.events(), rig.launched());
    let (source, target) = (snapshot(&from), snapshot(&to));
    let pid = rig.tool_pid(pane, "claude");
    assert!(pid.is_some(), "fixture running tool before Check");
    let (code, out, err) = reseat(rig, "fake-claude-b");
    assert_eq!(code, Some(1), "R4 Check must refuse: out={out} err={err}");
    assert!(out.is_empty(), "R4 refusal has no success output: {out}");
    assert!(
        err.contains("could not carry") && err.contains("nothing was stopped"),
        "R4 loud pre-stop refusal: {err}"
    );
    assert_eq!(
        rig.tool_pid(pane, "claude"),
        pid,
        "R4 running tool never stopped"
    );
    assert_eq!(
        (rig.meta(), rig.events(), rig.launched()),
        (meta, events, launches),
        "R4 no meta, journal or launch writes"
    );
    assert_eq!(snapshot(&from), source, "R4 no source writes");
    assert_eq!(snapshot(&to), target, "R4 no target writes or archives");
    assert!(
        !rig.dir.join("seed.scout.md").exists(),
        "R4 no seed from refused move"
    );
    err
}

#[test]
fn fork_transcript_refuses_before_stop_write_or_journal() {
    let rig = Rig::new("rrfork");
    let pane = plant(&rig, true);
    put(
        &account(&rig, "b").join(relative_transcript(&rig)),
        b"fork\n",
    );
    assert_refuses_untouched(&rig, &pane);
}

#[test]
fn longer_target_transcript_refuses_before_stop_write_or_journal() {
    let rig = Rig::new("rrlong");
    let pane = plant(&rig, true);
    put(
        &account(&rig, "b").join(relative_transcript(&rig)),
        &[TRANSCRIPT, b"target future\n"].concat(),
    );
    assert_refuses_untouched(&rig, &pane);
}

#[test]
fn equal_files_target_only_files_and_target_memory_are_untouched_without_new_receipts() {
    let rig = Rig::new("rrequal");
    let pane = plant(&rig, false);
    let to = account(&rig, "b");
    put(&to.join(relative_transcript(&rig)), TRANSCRIPT);
    for relative in sidecars(&rig) {
        put(&to.join(relative), b"source sidecar");
    }
    put(
        &to.join(&sidecars(&rig)[0])
            .with_file_name("target-only.txt"),
        b"target-owned tool result",
    );
    let memory = relative_transcript(&rig)
        .parent()
        .expect("project")
        .join("memory/notes.md");
    put(&account(&rig, "a").join(&memory), b"source memory");
    put(&to.join(&memory), b"target memory");
    for node in snapshot(&to).iter().filter(|node| node.kind == "file") {
        old_mtime(&to.join(&node.relative));
    }
    let before = snapshot(&to);
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert_eq!(
        code,
        Some(0),
        "R1/R2 equal files converge: out={out} err={err}"
    );
    assert_resumed(&rig, &pane, &to, TRANSCRIPT);
    assert_eq!(
        snapshot(&to),
        before,
        "R2 equal, target-only and memory nodes unchanged, inode and mtime included"
    );
    assert_base_receipts(&rig, &pane, &out, &err, true);
}

fn assert_base_receipts(rig: &Rig, pane: &str, out: &str, err: &str, memory: bool) {
    let expected = format!(
        "carried conversation {ID} from {} to {}{}\nreseated scout (pane {pane}, slot spawned.0) to fake-claude-b, carried\n",
        account(rig, "a").display(),
        account(rig, "b").display(),
        if memory {
            " (the target already had a project memory; it was kept, never merged)"
        } else {
            ""
        }
    );
    assert_eq!(
        out.as_bytes(),
        expected.as_bytes(),
        "R5 nothing archived: base stdout byte-identical"
    );
    assert!(
        err.is_empty(),
        "R5 nothing archived: base stderr empty: {err}"
    );
    let journal = rig.events();
    let events: Vec<_> = journal
        .lines()
        .filter_map(|line| ae::events::Event::parse_line(line).ok())
        .filter(|event| event.action == "reseat")
        .collect();
    assert_eq!(events.len(), 1, "one base move record");
    let summary = format!(
        "reseated scout (pane {pane}, slot spawned.0) to fake-claude-b, carried [from fake-claude-a to fake-claude-b, conversation {ID}, carried]"
    );
    assert_eq!(
        events[0].summary.as_deref(),
        Some(summary.as_str()),
        "R5 no archive: base move summary byte-identical"
    );
    let expected_record = format!(
        "{{\"ts\":\"{}\",\"actor\":\"lead\",\"action\":\"reseat\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_pane\":\"{pane}\",\"caller_pane\":\"{}\",\"summary\":\"{summary}\",\"schema_version\":1}}",
        events[0].ts, rig.main_pane
    );
    let raw_record = journal
        .lines()
        .find(|line| {
            ae::events::Event::parse_line(line).is_ok_and(|event| event.action == "reseat")
        })
        .expect("raw move record");
    assert_eq!(
        raw_record.as_bytes(),
        expected_record.as_bytes(),
        "R5 nothing archived: full base record byte-identical apart from its live timestamp and pane IDs"
    );
}

#[test]
fn clean_carry_keeps_base_stdout_and_move_record_bytes() {
    let rig = Rig::new("rrclean");
    let pane = plant(&rig, false);
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert_eq!(code, Some(0), "fixture clean carry: {out} {err}");
    assert_base_receipts(&rig, &pane, &out, &err, false);
}

#[test]
fn target_transcript_link_is_refused_without_following_it() {
    let rig = Rig::new("rrtlink");
    let pane = plant(&rig, true);
    let target = account(&rig, "b").join(relative_transcript(&rig));
    let outside = root(&rig).join("outside-transcript");
    put(&outside, STALE);
    std::fs::create_dir_all(target.parent().expect("target parent"))
        .expect("fixture target project");
    std::os::unix::fs::symlink(&outside, &target).expect("fixture target link");
    assert_refuses_untouched(&rig, &pane);
    assert_eq!(std::fs::read(outside).expect("outside untouched"), STALE);
}

#[test]
fn target_sidecar_leaf_link_is_refused_without_following_it() {
    let rig = Rig::new("rrslink");
    let pane = plant(&rig, true);
    let target = account(&rig, "b").join(&sidecars(&rig)[1]);
    let outside = root(&rig).join("outside-sidecar");
    put(&outside, b"outside sidecar");
    std::fs::create_dir_all(target.parent().expect("target parent"))
        .expect("fixture sidecar directory");
    std::os::unix::fs::symlink(&outside, &target).expect("fixture target link");
    assert_refuses_untouched(&rig, &pane);
    assert_eq!(
        std::fs::read(outside).expect("outside untouched"),
        b"outside sidecar"
    );
}

#[test]
fn target_sidecar_ancestor_link_is_refused_without_following_it() {
    let rig = Rig::new("rralink");
    let pane = plant(&rig, true);
    let outside = root(&rig).join("outside-tree");
    put(&outside.join(ID).join("h@v1"), b"outside sidecar");
    let before = snapshot(&outside);
    std::os::unix::fs::symlink(&outside, account(&rig, "b").join("file-history"))
        .expect("fixture linked ancestor");
    assert_refuses_untouched(&rig, &pane);
    assert_eq!(
        snapshot(&outside),
        before,
        "R6 ancestor link never followed"
    );
}

#[test]
fn a_directory_where_a_sidecar_file_belongs_is_still_refused() {
    let rig = Rig::new("rrshape");
    let pane = plant(&rig, true);
    std::fs::create_dir_all(account(&rig, "b").join(&sidecars(&rig)[1]))
        .expect("fixture wrong shape");
    assert_refuses_untouched(&rig, &pane);
}

#[test]
fn an_unreadable_target_sidecar_is_refused_before_stop() {
    let rig = Rig::new("rrclosed");
    let pane = plant(&rig, true);
    let closed = account(&rig, "b").join(&sidecars(&rig)[1]);
    put(&closed, b"closed target");
    let before = snapshot(&account(&rig, "b"));
    let permissions = std::fs::metadata(&closed)
        .expect("fixture permissions")
        .permissions();
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000))
        .expect("fixture unreadable file");
    let (meta, events, launches) = (rig.meta(), rig.events(), rig.launched());
    let pid = rig.tool_pid(&pane, "claude");
    assert!(pid.is_some(), "fixture running tool");
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    std::fs::set_permissions(&closed, permissions).expect("fixture restored for cleanup");
    assert_eq!(
        code,
        Some(1),
        "R2 unreadable is no regular-file proof: {out} {err}"
    );
    assert!(
        err.contains("nothing was stopped"),
        "R4 pre-stop refusal: {err}"
    );
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
    assert_eq!(
        (rig.meta(), rig.events(), rig.launched()),
        (meta, events, launches)
    );
    assert_eq!(
        std::fs::read(closed).expect("target preserved"),
        b"closed target"
    );
    assert_eq!(
        snapshot(&account(&rig, "b")),
        before,
        "R4 unreadable target creates no archive or copies"
    );
}

#[test]
fn an_archive_that_cannot_be_created_refuses_before_stopping_the_tool() {
    let rig = Rig::new("rrnowrite");
    let pane = plant(&rig, true);
    let to = account(&rig, "b");
    put(&to.join(relative_transcript(&rig)), STALE);
    let directories: Vec<_> = snapshot(&to)
        .into_iter()
        .filter(|node| node.kind == "dir")
        .collect();
    for node in &directories {
        std::fs::set_permissions(
            to.join(&node.relative),
            std::fs::Permissions::from_mode(0o500),
        )
        .expect("fixture entire account readonly: no possible archive parent");
    }
    assert_refuses_untouched(&rig, &pane);
    for node in directories {
        std::fs::set_permissions(
            to.join(node.relative),
            std::fs::Permissions::from_mode(node.mode),
        )
        .expect("fixture restored for cleanup");
    }
}

/// An external account writer runs at the actual stop boundary, after Check.
/// The pane starts this tiny Perl program, which execs the usual shell. Once
/// armed, tmux's real respawn re-runs that program before returning to the
/// shell the stop must prove. No carry method is mocked; no sleep decides
/// when to mutate the target.
fn change_on_stop(rig: &Rig, target: &Path, bytes: &[u8]) -> (String, PathBuf) {
    let script = root(rig).join("mutate-account.pl");
    let marker = root(rig).join("mutation-ran");
    let armed = root(rig).join("mutation-armed");
    let hex = bytes.iter().fold(String::new(), |mut hex, byte| {
        write!(hex, "{byte:02x}").expect("fixture hex bytes");
        hex
    });
    std::fs::write(&script, format!("if (-e '{}') {{ open(my $f, '>', '{}') or die; binmode($f); print $f pack('H*', '{hex}'); close($f) or die; open(my $m, '>', '{}') or die; print $m 'ran'; close($m); unlink('{}') or die; }} exec('/bin/sh') or die;\n", armed.display(), target.display(), marker.display(), armed.display())).expect("external writer script");
    (format!("/usr/bin/perl {}", script.display()), marker)
}

fn arm_change(rig: &Rig) {
    std::fs::write(root(rig).join("mutation-armed"), b"armed")
        .expect("fixture writer armed after initial pane startup");
}

fn assert_seeded_drift(rig: &Rig, pane: &str, out: &str, err: &str) {
    assert!(
        err.contains("could not carry")
            && err.contains("fresh conversation")
            && err.contains("seed pack"),
        "R4 changed archive decision abandons carry loudly: {err}"
    );
    assert!(
        !out.contains("carried conversation") && !out.contains("archiv"),
        "R4 no false carry/archive receipt: {out}"
    );
    assert_ne!(
        rig.meta_row("harness_session.spawned.0"),
        ID,
        "R4 fallback creates a fresh conversation"
    );
    assert!(
        rig.meta_row("harness_session_prior.spawned.0").contains(ID),
        "R6 original id remains predecessor"
    );
    assert!(
        rig.dir.join("seed.scout.md").exists(),
        "R4 today's seeded fallback"
    );
    assert!(
        rig.events().contains(&format!("prior {ID}, seeded (")),
        "R4 truthful fallback record: {}",
        rig.events()
    );
    assert!(
        rig.tool_pid(pane, "claude").is_some(),
        "seeded successor running"
    );
}

#[test]
fn an_equal_transcript_changed_to_a_fork_at_stop_uses_the_existing_seeded_fallback() {
    let rig = Rig::new("rrhookguard");
    let target = account(&rig, "b").join(relative_transcript(&rig));
    let changed = b"fork written by target account during stop\n";
    let (command, marker) = change_on_stop(&rig, &target, changed);
    let pane = plant_with_command(&rig, true, &command);
    put(&target, TRANSCRIPT);
    arm_change(&rig);
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert!(
        marker.is_file(),
        "fixture writer reached actual stop boundary: {out} {err}"
    );
    assert_eq!(code, Some(0), "R6 unchanged seeded fallback: {out} {err}");
    assert_seeded_drift(&rig, &pane, &out, &err);
    assert_eq!(std::fs::read(target).expect("fork preserved"), changed);
}

#[test]
fn a_stale_transcript_changed_to_another_prefix_after_check_is_not_archived() {
    let rig = Rig::new("rrtdrift");
    let to = account(&rig, "b");
    let target = to.join(relative_transcript(&rig));
    let changed = b"user\nassistant\n";
    let (command, marker) = change_on_stop(&rig, &target, changed);
    let pane = plant_with_command(&rig, true, &command);
    put(&target, STALE);
    arm_change(&rig);
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert!(
        marker.is_file(),
        "R4 Check accepts stale prefix and reaches real stop: code={code:?} out={out} err={err}"
    );
    assert_eq!(
        code,
        Some(0),
        "R4 Copy drift uses today's successful seeded move: {out} {err}"
    );
    assert_seeded_drift(&rig, &pane, &out, &err);
    assert_eq!(
        std::fs::read(target).expect("changed target remains"),
        changed,
        "R4 changed node is not archived or replaced"
    );
    assert_eq!(
        snapshot(&to)
            .iter()
            .filter(|node| node.kind == "file")
            .count(),
        1,
        "R4 no archive or sidecar writes before drift refusal"
    );
}

#[test]
fn a_conflicting_sidecar_changed_after_check_is_not_archived() {
    let rig = Rig::new("rrsdrift");
    let to = account(&rig, "b");
    let target = to.join(&sidecars(&rig)[0]);
    let changed = b"account writer changed sidecar";
    let (command, marker) = change_on_stop(&rig, &target, changed);
    let pane = plant_with_command(&rig, true, &command);
    put(&to.join(relative_transcript(&rig)), TRANSCRIPT);
    put(&target, b"old target sidecar");
    arm_change(&rig);
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert!(
        marker.is_file(),
        "R4 Check accepts archiveable sidecar and reaches real stop: code={code:?} out={out} err={err}"
    );
    assert_eq!(code, Some(0), "R4 Copy drift uses seeded move: {out} {err}");
    assert_seeded_drift(&rig, &pane, &out, &err);
    assert_eq!(
        std::fs::read(target).expect("changed target remains"),
        changed,
        "R4 changed node is not archived or replaced"
    );
    assert_eq!(
        snapshot(&to)
            .iter()
            .filter(|node| node.kind == "file")
            .count(),
        2,
        "R4 no archive before sidecar drift refusal"
    );
}

// ADD-only v1.1: R6 memory, R4 whole-plan/diagnostic proof, and the UTC leaf
// grammar approved in the round-trip docs. Frozen v1 assertions stay intact.

#[test]
fn v11_kept_target_memory_does_not_read_the_sources_memory() {
    let rig = Rig::new("rr11kept");
    let pane = plant(&rig, true);
    let project = relative_transcript(&rig)
        .parent()
        .expect("project directory")
        .to_path_buf();
    let source_memory = account(&rig, "a").join(&project).join("memory");
    let target_memory = account(&rig, "b").join(&project).join("memory");
    let outside = root(&rig).join("outside-memory");
    put(&outside, b"source memory behind a forbidden link");
    std::fs::create_dir_all(&source_memory).expect("fixture source memory");
    std::os::unix::fs::symlink(&outside, source_memory.join("notes.md"))
        .expect("fixture unreadable copy-set shape");
    put(&target_memory.join("notes.md"), b"target owns this memory");
    let before = snapshot(&target_memory);
    let source = snapshot(&account(&rig, "a"));

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert_eq!(
        code,
        Some(0),
        "R6 kept memory needs no source read: {out} {err}"
    );
    assert_resumed(&rig, &pane, &account(&rig, "b"), TRANSCRIPT);
    assert_eq!(
        snapshot(&target_memory),
        before,
        "R6 target memory untouched"
    );
    assert_eq!(snapshot(&account(&rig, "a")), source, "R6 source untouched");
    assert!(!account(&rig, "b").join("ae-carry-archive").exists());
    assert!(!out.contains("archiv"), "R5 no archive receipt: {out}");
    assert_eq!(
        std::fs::read(outside).expect("outside preserved"),
        b"source memory behind a forbidden link"
    );
}

#[test]
fn v11_missing_target_memory_checks_the_source_before_stopping() {
    let rig = Rig::new("rr11missing");
    let pane = plant(&rig, true);
    let project = relative_transcript(&rig)
        .parent()
        .expect("project directory")
        .to_path_buf();
    let memory = account(&rig, "a").join(project).join("memory");
    let outside = root(&rig).join("outside-memory");
    put(&outside, b"source memory behind a forbidden link");
    std::fs::create_dir_all(&memory).expect("fixture source memory");
    std::os::unix::fs::symlink(&outside, memory.join("notes.md"))
        .expect("fixture forbidden source memory node");

    assert_refuses_untouched(&rig, &pane);
    assert_eq!(
        std::fs::read(outside).expect("outside preserved"),
        b"source memory behind a forbidden link"
    );
}

#[test]
fn v11_a_later_changed_conflict_refuses_before_any_earlier_archive() {
    let rig = Rig::new("rr11allproof");
    let to = account(&rig, "b");
    let first = to.join(&sidecars(&rig)[1]);
    let later = to.join(&sidecars(&rig)[2]);
    let changed = b"account writer changed the later conflict";
    let (command, marker) = change_on_stop(&rig, &later, changed);
    let pane = plant_with_command(&rig, true, &command);
    put(&to.join(relative_transcript(&rig)), TRANSCRIPT);
    put(&first, b"first target conflict must stay here");
    put(&later, b"later target conflict before stop");
    let first_before = file_at(&to, &sidecars(&rig)[1]);
    arm_change(&rig);

    let (code, out, err) = reseat(&rig, "fake-claude-b");

    assert!(marker.is_file(), "fixture actual stop: {out} {err}");
    assert_eq!(code, Some(0), "R4 seeded fallback: {out} {err}");
    assert_seeded_drift(&rig, &pane, &out, &err);
    assert!(
        !to.join("ae-carry-archive").exists(),
        "R4 every frozen decision is re-proven before the first write"
    );
    assert_eq!(
        file_at(&to, &sidecars(&rig)[1]),
        first_before,
        "R4 earlier conflict is neither archived nor replaced"
    );
    assert_eq!(
        std::fs::read(later).expect("writer bytes retained"),
        changed
    );
    assert_eq!(
        snapshot(&to)
            .iter()
            .filter(|node| node.kind == "file")
            .count(),
        3,
        "R4 no partial copy set"
    );
}

#[test]
fn v11_changed_sidecar_is_named_in_stderr_and_the_seeded_record() {
    let rig = Rig::new("rr11cause");
    let to = account(&rig, "b");
    let target = to.join(&sidecars(&rig)[1]);
    let (command, marker) = change_on_stop(&rig, &target, b"changed account bytes");
    let pane = plant_with_command(&rig, true, &command);
    put(&to.join(relative_transcript(&rig)), TRANSCRIPT);
    put(&target, b"old account bytes");
    arm_change(&rig);
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert!(marker.is_file(), "fixture actual stop: {out} {err}");
    assert_eq!(code, Some(0), "R4 seeded fallback: {out} {err}");
    assert_seeded_drift(&rig, &pane, &out, &err);
    let path = target.display().to_string();
    let cause = "changed after ae checked it";
    assert!(
        err.contains(&path) && err.contains(cause),
        "R4 loud refusal names the changed path and cause: {err}"
    );
    let events = rig.events();
    assert!(
        events
            .lines()
            .filter_map(|line| ae::events::Event::parse_line(line).ok())
            .any(|event| event.action == "reseat"
                && event.summary.as_deref().is_some_and(|summary| summary
                    .starts_with("reseated ")
                    && summary.contains("seeded (")
                    && summary.contains(&path)
                    && summary.contains(cause))),
        "R4 seeded audit record names the same path and cause: {events}"
    );
}

#[test]
fn v11_archive_leaf_carries_the_documented_utc_second_pid_and_counter() {
    let rig = Rig::new("rr11utc");
    let pane = plant(&rig, false);
    let to = account(&rig, "b");
    let relative = relative_transcript(&rig);
    put(&to.join(&relative), STALE);
    let before = snapshot(&to);
    let old = file_at(&to, &relative);
    let (code, out, err) = reseat(&rig, "fake-claude-b");
    assert_eq!(code, Some(0), "R1 stale carries: {out} {err}");
    assert_resumed(&rig, &pane, &to, TRANSCRIPT);
    let archive = assert_archive(&to, &before, &[old], &out, &rig.events());
    let leaf = archive
        .file_name()
        .expect("archive leaf")
        .to_str()
        .expect("documented ASCII leaf");
    // Approved docs oracle: <YYYYMMDDTHHMMSSZ>-<pid>.<n>.
    let (stamp, unique) = leaf.split_once('-').expect("R3 UTC stamp followed by pid");
    assert_eq!(
        stamp.len(),
        16,
        "R3 eight date digits, T, six time digits, Z: {leaf}"
    );
    assert_eq!(stamp.as_bytes()[8], b'T', "R3 UTC date/time separator");
    assert_eq!(stamp.as_bytes()[15], b'Z', "R3 UTC zone");
    assert!(
        stamp.as_bytes()[..8]
            .iter()
            .chain(&stamp.as_bytes()[9..15])
            .all(u8::is_ascii_digit),
        "R3 UTC date/time digits: {leaf}"
    );
    let (pid, counter) = unique.split_once('.').expect("R3 pid and nonce counter");
    assert!(
        !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()),
        "R3 pid: {leaf}"
    );
    assert!(
        !counter.is_empty() && counter.bytes().all(|b| b.is_ascii_digit()),
        "R3 counter: {leaf}"
    );
}
