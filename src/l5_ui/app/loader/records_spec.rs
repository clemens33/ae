//! The reader's one-shot dialog read behind a card menu's `Activity…` and
//! `Memos…`: rows through the tmux session menu's own folds, under the same
//! identity rule, and nothing read for a record that is not the card's.
//!
//! These cases ask for a STOPPED card, whose identity reference is the
//! incarnation the card drew; a running card's live-option reference is
//! pinned end to end by `tests/it/app_menu_activity_memos_spec.rs`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{Answer, Dialog, Job, Reader, RecordsAsk, Request};
use crate::session_menu::{self, CellRow};

const CARD: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const OTHER: &str = "0199c0de-ffff-4890-abcd-ef0123456789";

/// A fresh owned directory under the process temp dir, holding a stopped
/// session's meta naming `uuid`.
fn session(name: &str, uuid: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae-records-load-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the fixture dir is made");
    std::fs::write(
        dir.join("meta"),
        format!("session_id={uuid}\nseat.main=lead\n"),
    )
    .expect("the meta is written");
    dir
}

/// A reader past its first fleet read, so `next` reaches the job.
fn reader() -> Reader {
    let mut reader = Reader::new(PathBuf::from("/nonexistent"), None, None);
    reader.fleet_at = Some(Instant::now());
    reader
}

#[allow(
    clippy::unnecessary_box_returns,
    reason = "the request carries the ask boxed, as the loader's Request does"
)]
fn ask(generation: u64, dialog: Dialog, uuid: &str, dir: &Path) -> Box<RecordsAsk> {
    Box::new(RecordsAsk {
        generation,
        dialog,
        session: "api".to_owned(),
        uuid: uuid.to_owned(),
        dir: dir.to_owned(),
        live: false,
    })
}

fn answered(reader: &mut Reader, asked: &RecordsAsk) -> Vec<CellRow> {
    let Answer::Records { generation, rows } = reader.records(asked) else {
        panic!("a records read answers records");
    };
    assert_eq!(generation, asked.generation);
    rows
}

fn gap(text: &str) -> Vec<CellRow> {
    vec![CellRow::Gap(text.to_owned())]
}

#[test]
fn a_taken_ask_is_due_at_once_answered_once_and_then_gone() {
    let dir = session("once", CARD);
    std::fs::write(
        dir.join("events.jsonl"),
        "{\"ts\":\"2020-10-30T10:20:00Z\",\"actor\":\"lead\",\"action\":\"goal\",\"target\":\"\",\"ref\":\"\",\"summary\":\"ROW_ONE\"}\n",
    )
    .expect("the events are written");
    let mut reader = reader();
    let asked = ask(7, Dialog::Activity, CARD, &dir);
    reader.take(Request::Records(Some(asked.clone())));
    assert!(
        matches!(reader.next(Instant::now()), Job::Records(_)),
        "due at once"
    );
    let rows = answered(&mut reader, &asked);
    let [CellRow::Cells(cells)] = rows.as_slice() else {
        panic!("one record, one row: {rows:?}");
    };
    assert_eq!(cells[1..], ["lead", "goal", "ROW_ONE"]);
    assert!(
        !matches!(reader.next(Instant::now()), Job::Records(_)),
        "one shot: a later next never re-reads"
    );
    // An abandoned ask is dropped unread; a second replaces an unread first.
    reader.take(Request::Records(Some(asked.clone())));
    reader.take(Request::Records(None));
    assert!(!matches!(reader.next(Instant::now()), Job::Records(_)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_dialog_is_the_tmux_dialogs_own_fold_of_the_same_bytes() {
    let dir = session("parity", CARD);
    let memo = "2020-10-30T10:00:00Z\tlead\tarch\tOLD\n2020-10-30T10:10:00Z\tlead\tarch\tNEW\n";
    std::fs::write(dir.join("memo.tsv"), memo).expect("the memos are written");
    let mut reader = reader();
    let rows = answered(&mut reader, &ask(1, Dialog::Memos, CARD, &dir));
    let meta = session_menu::MetaSource::read(&dir);
    let option = crate::tmux::OptionReading::Set(CARD.to_owned());
    let source = crate::store::SourceRead::Ready(memo.as_bytes().to_vec());
    let want = session_menu::memo_cells(&option, &meta, &source, crate::time::Timestamp::now());
    let [CellRow::Cells(got)] = rows.as_slice() else {
        panic!("one topic, one row: {rows:?}");
    };
    let [CellRow::Cells(want)] = want.as_slice() else {
        panic!("one topic, one row: {want:?}");
    };
    assert_eq!(got[1..], want[1..], "topic and latest text");
    assert_eq!(got[1..], ["arch", "NEW"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_or_unreadable_source_is_a_named_gap() {
    let dir = session("gaps", CARD);
    let mut reader = reader();
    assert_eq!(
        answered(&mut reader, &ask(1, Dialog::Activity, CARD, &dir)),
        gap("activity: none")
    );
    assert_eq!(
        answered(&mut reader, &ask(2, Dialog::Memos, CARD, &dir)),
        gap("memos: none")
    );
    std::fs::create_dir(dir.join("events.jsonl")).expect("a directory where a file belongs");
    let rows = answered(&mut reader, &ask(3, Dialog::Activity, CARD, &dir));
    assert!(
        matches!(rows.as_slice(), [CellRow::Gap(text)] if text.starts_with("activity: unreadable")),
        "{rows:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_record_of_another_incarnation_is_never_read_into_a_row() {
    let dir = session("foreign", OTHER);
    std::fs::write(
        dir.join("events.jsonl"),
        "{\"ts\":\"2020-10-30T10:20:00Z\",\"actor\":\"lead\",\"action\":\"goal\",\"target\":\"\",\"ref\":\"\",\"summary\":\"FOREIGN\"}\n",
    )
    .expect("the events are written");
    std::fs::write(
        dir.join("memo.tsv"),
        "2020-10-30T10:00:00Z\tlead\tleak\tFOREIGN\n",
    )
    .expect("the memos are written");
    let mut reader = reader();
    // The card drew CARD; the record now names OTHER.
    assert_eq!(
        answered(&mut reader, &ask(1, Dialog::Activity, CARD, &dir)),
        gap("activity: unavailable (meta: identity mismatch)")
    );
    assert_eq!(
        answered(&mut reader, &ask(2, Dialog::Memos, CARD, &dir)),
        gap("memos: unavailable (meta: identity mismatch)")
    );
    // A card that drew no identity has none to match.
    assert_eq!(
        answered(&mut reader, &ask(3, Dialog::Memos, "", &dir)),
        gap("memos: unavailable (session identity not recorded)")
    );
    // A record that is gone or damaged says so.
    let _ = std::fs::remove_file(dir.join("meta"));
    assert_eq!(
        answered(&mut reader, &ask(4, Dialog::Activity, OTHER, &dir)),
        gap("activity: unavailable (meta: missing)")
    );
    std::fs::write(
        dir.join("meta"),
        format!("session_id={OTHER}\nsession_id={OTHER}\n"),
    )
    .expect("a doubled identity");
    assert_eq!(
        answered(&mut reader, &ask(5, Dialog::Activity, OTHER, &dir)),
        gap("activity: unavailable (meta: duplicate identity)")
    );
    let _ = std::fs::remove_dir_all(&dir);
}
