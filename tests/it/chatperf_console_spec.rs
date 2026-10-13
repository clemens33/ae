//! Frozen chatperf acceptance, lead rulings 2026-10-13: the journal is read
//! whole once, then only past the last complete record while it is the SAME
//! file (dev, ino), not shrunk, and every byte already read is unchanged; any
//! other change, or any doubt, is a whole read. No record is hidden: every
//! read draws the lane a fresh whole read draws. A body is read once per name
//! and store file identity (dev, ino, len, mtime and ctime to the nanosecond,
//! mode); a changed key reads it again and a vanished file reads as before.
#![allow(clippy::disallowed_methods, clippy::expect_used, reason = "fixture")]
use super::{Console, tests::Rig};
use crate::events::parsed_bytes;
use crate::{provenance, tracked};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::PathBuf;

const REWRITTEN: &str = "-- journal rewritten";
pub(super) const CARD: &str = r#"{"ts":"2026-09-30T06:00:00Z","actor":"lead","action":"state","ref":"waiting-user","summary":"ship it?"}"#;
const WORK: &str = r#"{"ts":"2026-09-30T06:09:00Z","actor":"lead","action":"state","ref":"working","summary":"go"}"#;

pub(super) fn path(rig: &Rig) -> PathBuf {
    rig.0.join("s/events.jsonl")
}

/// Replace the journal's bytes IN PLACE: same inode, truncated then written.
pub(super) fn write(rig: &Rig, bytes: &[u8]) {
    fs::write(path(rig), bytes).expect("journal");
}

pub(super) fn append(rig: &Rig, bytes: &[u8]) {
    OpenOptions::new()
        .append(true)
        .open(path(rig))
        .and_then(|mut file| file.write_all(bytes))
        .expect("append");
}

fn len(rig: &Rig) -> u64 {
    fs::metadata(path(rig)).expect("journal").len()
}

fn ino(rig: &Rig) -> u64 {
    fs::metadata(path(rig)).expect("journal").ino()
}

/// Console ask `n` as one complete record: its own id and words.
pub(super) fn asked(n: u8, words: &str) -> String {
    format!(
        "{{\"ts\":\"2026-09-30T06:0{n}:00Z\",\"actor\":\"console:local\",\"action\":\"ask\",\"target\":\"lead\",\"ref\":\"ae-20260930T06000{n}Z-0000abcd\",\"summary\":\"{words}\"}}\n"
    )
}

/// `count` complete records the lane never draws, about 600 bytes each.
pub(super) fn filler(count: usize) -> String {
    let words = "z".repeat(520);
    (0..count)
        .map(|n| format!("{{\"ts\":\"2026-09-30T05:00:00Z\",\"actor\":\"w{n}\",\"action\":\"send\",\"target\":\"lead\",\"summary\":\"{words}\"}}\n"))
        .collect::<Vec<_>>()
        .concat()
}

/// `run`'s answer and the bytes the journal parser was handed meanwhile.
pub(super) fn parsed<T>(run: impl FnOnce() -> T) -> (T, u64) {
    let before = parsed_bytes();
    let answer = run();
    (answer, parsed_bytes().saturating_sub(before))
}

fn pass(console: &mut Console) -> (String, u64) {
    parsed(|| console.pass().expect("a pass"))
}

#[test]
fn an_unchanged_journal_parses_nothing_and_an_append_parses_only_its_own_bytes() {
    let rig = Rig::new("chatperf-append");
    let mut console = rig.console();
    write(
        &rig,
        format!("{}{}", filler(40), asked(1, "alpha")).as_bytes(),
    );
    let (first, cold) = pass(&mut console);
    assert!(first.contains("alpha"), "{first}");
    assert_eq!(cold, len(&rig), "the first read is whole");
    let (quiet, idle) = pass(&mut console);
    assert_eq!(quiet, "", "nothing new prints");
    assert_eq!(idle, 0, "an unchanged journal parses nothing");
    let two = asked(2, "bravo");
    append(&rig, two.as_bytes());
    let (next, warm) = pass(&mut console);
    assert!(
        next.contains("bravo") && !next.contains(REWRITTEN),
        "{next}"
    );
    assert_eq!(
        warm,
        two.len() as u64,
        "an append parses its own bytes, no others"
    );
}

#[test]
fn a_torn_record_waits_and_prints_once_whole_without_rereading_the_prefix() {
    let rig = Rig::new("chatperf-torn");
    let mut console = rig.console();
    write(
        &rig,
        format!("{}{}", filler(40), asked(1, "alpha")).as_bytes(),
    );
    pass(&mut console);
    let two = asked(2, "bravo");
    let (head, tail) = two.split_at(two.len() / 2);
    append(&rig, head.as_bytes());
    let (torn, cost) = pass(&mut console);
    assert!(!torn.contains("bravo"), "a torn record waits: {torn}");
    assert!(cost <= head.len() as u64, "only the torn bytes: {cost}");
    append(&rig, tail.as_bytes());
    let (whole, cost) = pass(&mut console);
    assert_eq!(whole.matches("bravo").count(), 1, "{whole}");
    assert!(!whole.contains(REWRITTEN), "{whole}");
    assert!(
        cost <= two.len() as u64,
        "only the completed record: {cost}"
    );
    assert_eq!(pass(&mut console), (String::new(), 0));
}

#[test]
fn an_in_place_rewrite_that_grows_is_read_whole_and_hides_no_record() {
    let rig = Rig::new("chatperf-grow");
    let mut console = rig.console();
    let pad = filler(1200);
    let one = asked(1, "alpha");
    write(
        &rig,
        format!("{pad}{one}{pad}{}", asked(2, "bravo")).as_bytes(),
    );
    let (first, _) = pass(&mut console);
    assert!(
        first.contains("alpha") && first.contains("bravo"),
        "{first}"
    );
    let five = asked(5, "ALPHA");
    assert_eq!(five.len(), one.len(), "the middle record keeps its length");
    let at = ino(&rig);
    let grown = format!(
        "{pad}{five}{pad}{}{}",
        asked(2, "bravo"),
        asked(3, "charlie")
    );
    write(&rig, grown.as_bytes());
    assert_eq!(ino(&rig), at, "rewritten in place, not replaced");
    let (got, cost) = pass(&mut console);
    assert!(got.contains(REWRITTEN), "{got}");
    assert!(got.contains("ALPHA") && got.contains("charlie"), "{got}");
    assert_eq!(cost, len(&rig), "a rewrite in place is read whole");
}

#[test]
fn a_shrunk_or_replaced_journal_is_read_whole() {
    let rig = Rig::new("chatperf-whole");
    let mut console = rig.console();
    let (one, two, three) = (asked(1, "alpha"), asked(2, "bravo"), asked(3, "charlie"));
    write(&rig, format!("{}{one}{two}", filler(40)).as_bytes());
    pass(&mut console);
    write(&rig, format!("{}{two}", filler(40)).as_bytes());
    let (shrunk, cost) = pass(&mut console);
    assert!(shrunk.contains(REWRITTEN), "{shrunk}");
    assert_eq!(cost, len(&rig), "a shrunk journal is read whole");
    let same = fs::read(path(&rig)).expect("journal");
    let staged = rig.0.join("s/staged");
    fs::write(&staged, [same.as_slice(), three.as_bytes()].concat()).expect("staged");
    let at = ino(&rig);
    fs::rename(&staged, path(&rig)).expect("replace");
    assert_ne!(ino(&rig), at, "a new file");
    let (replaced, cost) = pass(&mut console);
    assert!(
        replaced.contains("charlie") && !replaced.contains(REWRITTEN),
        "{replaced}"
    );
    assert_eq!(
        cost,
        len(&rig),
        "a replaced journal is read whole, even one that only grew"
    );
}

#[test]
fn a_journal_that_cannot_be_read_now_is_named_unreadable_never_answered_from_before() {
    let rig = Rig::new("chatperf-unreadable");
    let mut console = rig.console();
    write(&rig, format!("{CARD}\n").as_bytes());
    let (first, _) = pass(&mut console);
    assert!(first.contains("DECISION lead"), "{first}");
    append(&rig, format!("{WORK}\n").as_bytes());
    fs::set_permissions(path(&rig), fs::Permissions::from_mode(0o000)).expect("chmod");
    if matches!(
        crate::store::read_source(&path(&rig)),
        crate::store::SourceRead::Unreadable(_)
    ) {
        let (blind, _) = pass(&mut console);
        assert!(blind.contains("journal — unreadable"), "{blind}");
        assert!(
            !blind.contains("closed"),
            "an unread journal closes no card: {blind}"
        );
    }
    fs::set_permissions(path(&rig), fs::Permissions::from_mode(0o600)).expect("chmod");
    let (back, _) = pass(&mut console);
    assert!(back.contains("-- closed: DECISION lead"), "{back}");
    assert!(!back.contains(REWRITTEN), "{back}");
}

/// What a read drew from the journal: its rows, the coverage after the
/// board's own (which a follow names once), and whether it settled.
fn drawn(read: super::Read) -> (Vec<super::lane::Item>, Vec<String>, bool) {
    let journal = read.lane.coverage[read.board_gaps..].to_vec();
    (read.lane.items, journal, read.settled)
}

/// The oracle is today's whole read: a console opened fresh at each step.
#[test]
fn every_read_draws_the_lane_a_fresh_whole_read_draws() {
    let rig = Rig::new("chatperf-oracle");
    let mut console = rig.console();
    let fresh = |rig: &Rig| drawn(rig.console().read().expect("a fresh read"));
    let same = |console: &mut Console, step: &str| {
        let read = drawn(console.read().expect("a read"));
        assert_eq!(read, fresh(&rig), "step {step}");
    };
    let (one, two) = (asked(1, "alpha"), asked(2, "bravo"));
    write(&rig, format!("{CARD}\n{}{one}", filler(20)).as_bytes());
    same(&mut console, "first");
    append(&rig, b"\r\nnot-json\n\xff\xfe\n\n");
    append(&rig, two.trim_end().as_bytes());
    append(&rig, b"\r\n");
    same(&mut console, "damage, a blank line and CRLF");
    let wide = asked(3, "na\u{ef}ve\u{2014}");
    let cut = wide.find('\u{ef}').expect("a two-byte character") + 1;
    append(&rig, &wide.as_bytes()[..cut]);
    same(&mut console, "torn inside a character");
    append(&rig, &wide.as_bytes()[cut..]);
    same(&mut console, "completed");
    let work = format!("{WORK}\n");
    append(&rig, &work.as_bytes()[..work.len() / 2]);
    same(&mut console, "torn closing record");
    append(&rig, &work.as_bytes()[work.len() / 2..]);
    same(&mut console, "closing record");
    let now = fs::read(path(&rig)).expect("journal");
    let at = now
        .windows(5)
        .position(|w| w == b"alpha")
        .expect("alpha is in the journal");
    let mut edited = now.clone();
    edited[at..at + 5].copy_from_slice(b"ALPHA");
    edited.extend_from_slice(asked(4, "delta").as_bytes());
    write(&rig, &edited);
    same(&mut console, "in-place edit that grows");
    write(&rig, format!("{CARD}\n").as_bytes());
    same(&mut console, "shrunk");
    let staged = rig.0.join("s/staged");
    fs::write(&staged, format!("{CARD}\n{}", asked(6, "foxtrot"))).expect("staged");
    fs::rename(&staged, path(&rig)).expect("replace");
    same(&mut console, "replaced");
    fs::remove_file(path(&rig)).expect("remove");
    same(&mut console, "absent");
    write(&rig, asked(7, "golf").as_bytes());
    same(&mut console, "recreated");
}

/// The console ask `REQ` whose stored body is `name` under `messages/`.
const REQ: &str = "ae-20260930T060000Z-0000abcd";

fn body_ask(name: &str) -> String {
    format!(
        "{{\"ts\":\"2026-09-30T06:00:00Z\",\"actor\":\"console:local\",\"action\":\"ask\",\"target\":\"lead\",\"ref\":\"{REQ}\",\"target_slot\":\"main\",\"target_session\":\"s\",\"body_file\":\"/x/messages/{name}\",\"summary\":\"SUMMARY\"}}\n"
    )
}

fn envelope(body: &str) -> String {
    provenance::first_line(
        &provenance::peer(tracked::CONSOLE_DISPLAY),
        &tracked::compose(
            tracked::Kind::Ask,
            REQ,
            tracked::CONSOLE_DISPLAY,
            body,
            "/fixture/reply \"<your reply>\"",
        ),
    )
}

thread_local! {
    static BODY_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Count one body file [`super::body_for`] reads, on this thread.
pub(super) fn count_body_read() {
    BODY_READS.with(|reads| reads.set(reads.get().saturating_add(1)));
}

/// Whether `console`'s next read shows `words` in a row, and how many body
/// files that read opened.
fn shows(console: &mut Console, words: &str) -> (bool, u64) {
    let before = BODY_READS.with(std::cell::Cell::get);
    let lane = console.read().expect("a read").lane;
    let reads = BODY_READS.with(std::cell::Cell::get).saturating_sub(before);
    (
        lane.items.iter().any(|item| item.body.contains(words)),
        reads,
    )
}

/// The memo key is the store's file identity (dev, ino, len, mtime and ctime
/// to the nanosecond, mode): any write moves ctime, so any change reads again.
#[test]
fn a_body_is_read_once_while_its_file_identity_holds_and_again_on_any_change() {
    let rig = Rig::new("chatperf-body");
    let messages = rig.0.join("s/messages");
    fs::create_dir(&messages).expect("messages");
    let name = format!("{REQ}.ask.00000a.txt");
    let file = messages.join(&name);
    write(&rig, body_ask(&name).as_bytes());
    fs::write(&file, envelope("first words")).expect("body");
    let stamp = fs::metadata(&file)
        .and_then(|meta| meta.modified())
        .expect("mtime");
    let mut console = rig.console();
    assert_eq!(
        shows(&mut console, "first words"),
        (true, 1),
        "the first read reads it"
    );
    assert_eq!(
        shows(&mut console, "first words"),
        (true, 0),
        "an unchanged body is not read again"
    );
    fs::write(&file, envelope("other words")).expect("same length, same inode");
    OpenOptions::new()
        .write(true)
        .open(&file)
        .and_then(|body| body.set_modified(stamp))
        .expect("the same mtime");
    assert_eq!(
        shows(&mut console, "other words"),
        (true, 1),
        "the same length and mtime but a new ctime reads again"
    );
    fs::write(&file, envelope("other words!")).expect("a new length");
    assert_eq!(
        shows(&mut console, "other words!"),
        (true, 1),
        "a new length reads again"
    );
    fs::remove_file(&file).expect("vanish");
    let gone = console.read().expect("a read").lane;
    let fresh = rig.console().read().expect("fresh").lane;
    assert_eq!(gone.items, fresh.items, "a vanished body reads as today");
    fs::write(&file, envelope("back again")).expect("reappears");
    assert_eq!(
        shows(&mut console, "back again"),
        (true, 1),
        "a body that reappears is read"
    );
}
