//! Frozen acceptance (#248 P1a): a real `_end` and a real reboot archive carry the
//! purge record and say which operation archived them.
//!
//! Oracle: #248 (record kept at end, read only by purge) and lead ruling I4
//! (archive `reason` end|reboot). Real `_end`/`_compact-*`, private tmux socket.

use std::fmt::Write as _;

use super::*;

const ID: &str = "44444444-4444-4444-8444-444444444444";

fn archive_of(rig: &Rig) -> PathBuf {
    rig.home.join("archive").join(UUID)
}

fn reasons(meta: &str) -> Vec<&str> {
    meta.lines().filter(|l| l.starts_with("reason=")).collect()
}

fn with_conversation(rig: &Rig) {
    let meta = rig.dir.join("meta");
    let mut text = std::fs::read_to_string(&meta).expect("rig meta");
    let _ = writeln!(text, "harness_session.main={ID}");
    std::fs::write(&meta, text).expect("meta with a conversation");
}

#[test]
fn endrecord_end_archives_the_record_with_reason_end_and_the_work_dir() {
    let rig = Rig::new("er1");
    with_conversation(&rig);
    let (code, out, err) = rig.run(&["_end", "-f", &rig.name]);
    assert_eq!(code, Some(0), "end: {out}{err}");
    let target = archive_of(&rig);
    let bytes = std::fs::read(target.join(ae::archive::harness::FILE))
        .unwrap_or_else(|why| panic!("the ended archive keeps harness.tsv: {why}"));
    let record = ae::archive::harness::parse(&bytes, UUID).expect("the record parses");
    let main = record
        .slots
        .iter()
        .find(|slot| slot.slot == "main")
        .expect("main row");
    assert_eq!(main.profile, "fake");
    assert_eq!(main.binary, "sh");
    assert_eq!(
        main.current,
        Some(ae::archive::harness::Conversation {
            tool: "sh".to_owned(),
            id: ID.to_owned(),
        })
    );
    let meta = std::fs::read_to_string(target.join("meta")).expect("archive meta");
    assert_eq!(reasons(&meta), ["reason=end"], "{meta}");
    assert!(
        meta.lines()
            .any(|l| l == format!("source_work_dir={}", rig.home.display())),
        "{meta}"
    );
    assert!(
        !meta.contains(ID),
        "the id stays out of archive meta: {meta}"
    );
}

#[test]
fn endrecord_a_reboot_archive_says_reboot_and_keeps_the_record() {
    let rig = Rig::new("er2");
    with_conversation(&rig);
    assert!(
        rig.tmux(&["new-session", "-d", "-s", "keepalive", "sh"]).0,
        "a second session keeps the server answering"
    );
    assert!(rig.tmux(&["kill-session", "-t", &rig.name]).0, "stopped");
    let dir = rig.dir.to_str().unwrap();
    let (code, tuple, err) = rig.run(&["_compact-freeze", dir]);
    assert_eq!(code, Some(0), "freeze: {err}");
    let tuple = tuple.trim_end().to_owned();
    let at = "2026-08-01T00:00:00Z";
    let args = [
        "_compact-archive",
        dir,
        &tuple,
        at,
        "not-managed",
        "-",
        "-",
        "-",
    ];
    let (code, out, err) = rig.run(&args);
    assert_eq!(code, Some(0), "reboot archive: {out}{err}");
    let target = archive_of(&rig);
    let meta = std::fs::read_to_string(target.join("meta")).expect("archive meta");
    assert_eq!(reasons(&meta), ["reason=reboot"], "{meta}");
    let bytes = std::fs::read(target.join(ae::archive::harness::FILE))
        .unwrap_or_else(|why| panic!("a reboot archive keeps harness.tsv too: {why}"));
    assert!(ae::archive::harness::parse(&bytes, UUID).is_ok());
}
