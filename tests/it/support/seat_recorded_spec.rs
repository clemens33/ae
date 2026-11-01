//! Fresh main-seat fixture readiness, independent of the implementation.

use std::cell::Cell;
use std::io;
use std::time::{Duration, Instant};

use super::seat_recorded;

const META: &str = "profile.main=idle\nconfig_home.main=absent\n";
const EVENT: &str = concat!(
    "{\"ts\":\"2026-11-01T09:00:00Z\",\"actor\":\"watchdog\",",
    "\"action\":\"conversation-new\",\"target\":\"lead\",\"target_slot\":\"main\"}\n"
);

fn ready(file: &str) -> String {
    match file {
        "meta" => META.to_owned(),
        "events.jsonl" => EVENT.to_owned(),
        _ => panic!("unexpected fixture read: {file}"),
    }
}

fn failure(read: impl Fn(&str) -> io::Result<String>) -> String {
    let why = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        seat_recorded(read, Instant::now());
    }))
    .expect_err("incomplete or unreadable setup cannot become ready");
    why.downcast_ref::<String>()
        .cloned()
        .or_else(|| why.downcast_ref::<&str>().map(|text| (*text).to_owned()))
        .expect("a textual readiness failure")
}

#[test]
fn both_successful_file_reads_are_required_even_when_the_deadline_is_spent() {
    let meta = Cell::new(0);
    let events = Cell::new(0);
    seat_recorded(
        |file| {
            let count = match file {
                "meta" => &meta,
                "events.jsonl" => &events,
                _ => panic!("unexpected file: {file}"),
            };
            count.set(count.get() + 1);
            Ok(ready(file))
        },
        Instant::now(),
    );
    assert_eq!((meta.get(), events.get()), (1, 1));
}

#[test]
fn missing_or_unrelated_rows_are_not_a_main_seat_start() {
    let worker = EVENT.replace("\"main\"", "\"worker.0\"");
    let other = EVENT.replace("conversation-new", "state");
    let split = format!("{worker}{other}");
    for (meta, events, file) in [
        ("profile.main=idle\n", EVENT, "meta"),
        ("note=config_home.main=absent\n", EVENT, "meta"),
        (META, "", "events.jsonl"),
        (META, worker.as_str(), "events.jsonl"),
        (META, split.as_str(), "events.jsonl"),
    ] {
        let why = failure(|name| {
            Ok(match name {
                "meta" => meta.to_owned(),
                "events.jsonl" => events.to_owned(),
                _ => panic!("unexpected file: {name}"),
            })
        });
        for expected in [file, "ok=true", "1 read"] {
            assert!(why.contains(expected), "missing {expected}: {why}");
        }
    }
}

#[test]
fn an_io_error_cannot_supply_evidence_from_its_message() {
    for file in ["meta", "events.jsonl"] {
        let why = failure(|name| {
            if name == file {
                Err(io::Error::other(ready(name)))
            } else {
                Ok(ready(name))
            }
        });
        for expected in [file, "ok=false", "1 read"] {
            assert!(why.contains(expected), "missing {expected}: {why}");
        }
    }
}

#[test]
fn failed_and_unmet_reads_recover_then_stop_at_the_first_qualifying_read() {
    let meta = Cell::new(0);
    let events = Cell::new(0);
    seat_recorded(
        |file| {
            let count = match file {
                "meta" => &meta,
                "events.jsonl" => &events,
                _ => panic!("unexpected file: {file}"),
            };
            count.set(count.get() + 1);
            match count.get() {
                1 => Err(io::Error::other(ready(file))),
                2 => Ok(String::new()),
                3 => Ok(ready(file)),
                _ => panic!("read past the first proof: {file}"),
            }
        },
        Instant::now() + Duration::from_secs(30),
    );
    assert_eq!((meta.get(), events.get()), (3, 3));
}

#[test]
fn the_second_file_does_not_receive_a_fresh_deadline() {
    let events = Cell::new(0);
    let why = failure(|file| {
        if file == "events.jsonl" {
            events.set(events.get() + 1);
            if events.get() == 1 {
                return Ok("not the conversation decision\n".to_owned());
            }
        }
        Ok(ready(file))
    });
    assert_eq!(events.get(), 1, "the spent budget cannot buy another read");
    for expected in [
        "events.jsonl",
        "conversation-new",
        "not the conversation decision",
        "ok=true",
    ] {
        assert!(why.contains(expected), "missing {expected}: {why}");
    }
}
