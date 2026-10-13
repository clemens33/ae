//! Frozen chatperf acceptance, strict half (lead rulings 2026-10-13): the
//! reads submit and `/close` decide on take the SAME held proof as the follow
//! read and parse only what was appended since; every answer, refusals
//! included, is byte-identical to today's whole strict read. Cost is the one
//! parse tally: `EventLog::drain`'s and a strict read's `count_strict` alike.
#![allow(clippy::disallowed_methods, clippy::expect_used, reason = "fixture")]
use super::chatperf_spec::{CARD, append, asked, filler, parsed, path, write};
use super::tests::Rig;
use crate::events::{Event, Held};
use crate::store::read_source;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

type Strict = Result<Vec<Event>, String>;
/// What a case is called, the journal before, and what happens to it after.
type Case = (&'static str, String, Box<dyn FnOnce(&Path)>);

/// The strict read submit and `/close` decide on: from the held proof when
/// it holds, else today's whole strict read, the answer byte-identical either
/// way. `held` is primed by one strict read of `before`, then `after` is read.
fn strict(before: &[u8], after: impl FnOnce(&Path)) -> (Strict, Strict, u64) {
    let rig = Rig::new("chatperf-strict");
    let journal = path(&rig);
    fs::write(&journal, before).expect("journal");
    let mut held = Held::default();
    let primed = super::snapshot_from(&mut held, read_source(&journal));
    assert_eq!(
        primed,
        super::snapshot(read_source(&journal)),
        "a cold held read is the whole read"
    );
    after(&journal);
    let (got, cost) = parsed(|| super::snapshot_from(&mut held, read_source(&journal)));
    (got, super::snapshot(read_source(&journal)), cost)
}

/// Append `bytes` to the journal, the same file.
fn grow(bytes: impl AsRef<[u8]> + 'static) -> impl FnOnce(&Path) {
    move |journal| {
        OpenOptions::new()
            .append(true)
            .open(journal)
            .and_then(|mut file| file.write_all(bytes.as_ref()))
            .expect("append");
    }
}

#[test]
fn a_held_strict_read_parses_only_an_append_and_answers_as_the_whole_read() {
    let prefix = format!("{}\n{}", filler(40), asked(1, "alpha"));
    let two = asked(2, "bravo");
    let (got, whole, cost) = strict(prefix.as_bytes(), grow(two.clone()));
    assert_eq!(got, whole);
    assert!(got.is_ok(), "{got:?}");
    assert_eq!(
        cost,
        two.len() as u64,
        "an append parses its own bytes, no others"
    );
}

#[test]
fn every_held_strict_answer_to_damage_or_change_is_the_whole_reads() {
    let blanks = format!("{CARD}\n\n\n{}", asked(1, "alpha"));
    let damaged = format!("{CARD}\nnot-json\n{}", asked(1, "alpha"));
    let cases: [Case; 9] = [
        ("a bad line after blank lines names its absolute line", blanks.clone(), Box::new(grow(b"not-json\n"))),
        ("a torn tail", blanks.clone(), Box::new(grow(b"{\"ts\":\"2026"))),
        ("a tail torn inside a character", blanks.clone(), Box::new(grow(b"{\"summary\":\"na\xc3"))),
        ("a complete line that is not UTF-8", blanks.clone(), Box::new(grow(b"\xff\xfe\n"))),
        ("a prefix holding a bad line", damaged, Box::new(grow(b"\n"))),
        ("carriage returns", blanks.clone(), Box::new(grow(b"{\"ts\":\"2026-09-30T06:05:00Z\",\"actor\":\"lead\",\"action\":\"chat\",\"summary\":\"cr\"}\r\r\n"))),
        ("shrunk in place", blanks.clone(), Box::new(|journal: &Path| fs::write(journal, format!("{CARD}\n")).expect("shrink"))),
        ("absent", blanks.clone(), Box::new(|journal: &Path| fs::remove_file(journal).expect("remove"))),
        ("a symlink to the same bytes plus one record", blanks, Box::new(|journal: &Path| {
            let target = journal.with_file_name("target");
            let mut bytes = fs::read(journal).expect("journal");
            bytes.extend_from_slice(asked(2, "bravo").as_bytes());
            fs::write(&target, bytes).expect("target");
            fs::remove_file(journal).expect("remove");
            std::os::unix::fs::symlink(&target, journal).expect("link");
        })),
    ];
    for (case, before, after) in cases {
        let (got, whole, _) = strict(before.as_bytes(), after);
        assert_eq!(got, whole, "{case}");
    }
}

#[test]
fn a_held_strict_read_of_an_in_place_rewrite_that_grows_answers_as_the_whole_read() {
    let pad = filler(1200);
    let before = format!("{pad}{}{pad}{}", asked(1, "alpha"), asked(2, "bravo"));
    let grown = format!(
        "{pad}{}{pad}{}{}",
        asked(5, "ALPHA"),
        asked(2, "bravo"),
        asked(3, "charlie")
    );
    let (got, whole, _) = strict(before.as_bytes(), |journal| {
        let at = fs::metadata(journal).expect("journal").ino();
        fs::write(journal, grown.as_bytes()).expect("rewrite in place");
        assert_eq!(
            fs::metadata(journal).expect("journal").ino(),
            at,
            "the same file"
        );
    });
    assert!(
        whole
            .as_ref()
            .is_ok_and(|events| events.iter().any(|e| e.summary.as_deref() == Some("ALPHA")))
    );
    assert_eq!(got, whole, "the rewritten record, never the held one");
}

/// A console ask `id` to `lead`, recorded as delivered.
fn console_ask(id: &str) -> String {
    format!(
        "{{\"ts\":\"2026-09-30T06:00:00Z\",\"actor\":\"console:local\",\"action\":\"ask\",\"target\":\"lead\",\"ref\":\"{id}\",\"target_slot\":\"main\",\"target_session\":\"s\",\"summary\":\"q\"}}\n"
    )
}

#[test]
fn a_held_submit_parses_on_each_read_only_what_was_appended_before_it() {
    let rig = Rig::new("chatperf-submit");
    let dir = rig.0.join("s");
    write(&rig, filler(40).as_bytes());
    let mut held = Held::default();
    assert!(
        super::snapshot_from(&mut held, read_source(&path(&rig))).is_ok(),
        "primed"
    );
    let before = asked(1, "alpha");
    append(&rig, before.as_bytes());
    let id = "ae-20260930T060900Z-0000beef";
    let record = console_ask(id);
    let deliver = || {
        append(&rig, record.as_bytes());
        String::new()
    };
    let submit = super::submit::submit_held;
    let (sent, cost) = parsed(|| submit(&mut held, &dir, "s", b"raw", id, || Ok(()), deliver));
    assert_eq!(sent, Ok(super::submit::Outcome::Sent(id.to_owned(), None)));
    let appended = (before.len() + record.len()) as u64;
    assert_eq!(
        cost, appended,
        "each strict read parses only what was appended before it"
    );
}

#[test]
fn a_held_close_parses_only_what_was_appended_and_closes_as_today() {
    let rig = Rig::new("chatperf-close");
    let dir = rig.0.join("s");
    write(&rig, filler(40).as_bytes());
    let mut held = Held::default();
    assert!(
        super::snapshot_from(&mut held, read_source(&path(&rig))).is_ok(),
        "primed"
    );
    let id = "ae-20260930T060900Z-0000beef";
    let one = console_ask(id);
    append(&rig, one.as_bytes());
    let (closed, cost) = parsed(|| super::close_held(&mut held, &dir, "s", Some(id)));
    assert_eq!(closed.map(|closed| closed.id), Ok(id.to_owned()));
    assert_eq!(
        cost,
        one.len() as u64,
        "the close read parses only the appended ask"
    );
}
