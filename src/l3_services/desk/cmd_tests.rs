//! The `desk` helper's argv, its input reads, its show and the order in which
//! it proves the caller before it touches any input.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "this unit test owns and inspects its scratch files"
)]

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use super::{grammar_refusal, input, parse, run};
use crate::desk::{INPUT_CAP, ParseError, parse_input};
use crate::deskstore::set;
use crate::time::Timestamp;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let id = format!("ae-desk-cmd-{tag}-{}", std::process::id());
        let path = std::env::temp_dir().join(id);
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).expect("private scratch directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|word| (*word).to_owned()).collect()
}

/// A standard input that fails the test when anything reads it.
struct Untouched;

impl Read for Untouched {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("the helper read standard input before it proved the caller");
    }
}

#[test]
fn the_argv_is_show_set_or_a_usage_error() {
    let parsed = |list: &[&str]| parse(&words(list)).map(|command| format!("{command:?}"));
    assert_eq!(parsed(&[]).as_deref(), Some("Show"));
    assert_eq!(parsed(&["show"]).as_deref(), Some("Show"));
    assert_eq!(parsed(&["set"]).as_deref(), Some("Set(None)"));
    assert_eq!(parsed(&["set", "-"]).as_deref(), Some("Set(None)"));
    assert_eq!(
        parsed(&["set", "board.txt"]).as_deref(),
        Some("Set(Some(\"board.txt\"))")
    );
    for refused in [
        &["show", "x"][..],
        &["set", "a", "b"],
        &["set", "-", "x"],
        &["frob"],
        &["--force"],
        &[""],
    ] {
        assert_eq!(parse(&words(refused)), None, "{refused:?}");
    }
}

#[test]
fn a_usage_error_exits_two_with_the_grammar_on_stderr_and_nothing_on_stdout() {
    let dir = Scratch::new("usage");
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(
        &dir.0,
        &words(&["frob"]),
        &mut || None,
        Timestamp::from_epoch(1_795_164_012),
        &mut Untouched,
        &mut out,
        &mut err,
    )
    .expect("writes to memory");
    assert_eq!(code, 2);
    assert!(out.is_empty());
    let err = String::from_utf8(err).expect("utf-8");
    assert!(
        err.contains("desk show") && err.contains("desk set"),
        "{err}"
    );
}

#[test]
fn show_prints_no_board_set_then_the_input_form_and_names_a_damaged_record() {
    let dir = Scratch::new("show");
    let show = |dir: &PathBuf| {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(
            dir,
            &words(&["show"]),
            &mut || None,
            Timestamp::from_epoch(1_795_164_012),
            &mut Untouched,
            &mut out,
            &mut err,
        )
        .expect("writes to memory");
        (
            code,
            String::from_utf8(out).expect("utf-8"),
            String::from_utf8(err).expect("utf-8"),
        )
    };
    assert_eq!(
        show(&dir.0),
        (0, "no board set\n".to_owned(), String::new())
    );

    let items = parse_input(b"DECIDE\nD1 | Pick | https://example.test/a\nRUNNING\nR2 | Work\n")
        .expect("input");
    set(&dir.0, items, Timestamp::from_epoch(1_795_164_012), "boss").expect("set");
    assert_eq!(
        show(&dir.0),
        (
            0,
            "DECIDE\nD1 | Pick | https://example.test/a\nRUNNING\nR2 | Work\n".to_owned(),
            String::new()
        )
    );

    fs::write(dir.0.join("desk-record"), "not a record\n").expect("damage it");
    let (code, out, err) = show(&dir.0);
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(
        err.starts_with("desk show: the board record is damaged"),
        "{err}"
    );
}

#[test]
fn a_set_proves_the_caller_before_it_reads_any_input() {
    let dir = Scratch::new("order");
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(
        &dir.0,
        &words(&["set", "-"]),
        &mut || None,
        Timestamp::from_epoch(1_795_164_012),
        &mut Untouched,
        &mut out,
        &mut err,
    )
    .expect("writes to memory");
    assert_eq!(code, 1);
    assert!(out.is_empty());
    let err = String::from_utf8(err).expect("utf-8");
    assert!(err.starts_with("desk set REFUSED: "), "{err}");
    assert!(!dir.0.join("desk-record").exists());
    assert!(!dir.0.join("desk-record.lock").exists());
}

#[test]
fn a_file_is_read_whole_up_to_one_byte_past_the_cap_and_a_missing_one_is_named() {
    let dir = Scratch::new("file");
    let path = dir.0.join("board.txt");
    fs::write(&path, b"DECIDE\n").expect("input");
    let name = path.to_str().expect("path");
    assert_eq!(input(Some(name), &mut Untouched), Ok(b"DECIDE\n".to_vec()));

    fs::write(&path, vec![b' '; INPUT_CAP + 1]).expect("one byte over");
    assert_eq!(
        input(Some(name), &mut Untouched).map(|bytes| bytes.len()),
        Ok(INPUT_CAP + 1),
        "the grammar refuses it by name"
    );
    fs::write(&path, vec![b' '; INPUT_CAP + 2]).expect("two bytes over");
    let long = input(Some(name), &mut Untouched).expect_err("over the read bound");
    assert!(long.ends_with("is longer than 4096 bytes"), "{long}");

    let missing = dir.0.join("none.txt");
    let absent = input(missing.to_str(), &mut Untouched).expect_err("absent");
    assert!(absent.contains("no such file"), "{absent}");

    let directory = input(dir.0.to_str(), &mut Untouched).expect_err("a directory");
    assert!(directory.contains("not a regular file"), "{directory}");
}

#[test]
fn standard_input_is_read_to_one_byte_past_the_cap_and_no_further() {
    let mut endless = io::repeat(b'x');
    let bytes = input(None, &mut endless).expect("bounded");
    assert_eq!(bytes.len(), INPUT_CAP + 1);
    let mut short: &[u8] = b"RUNNING\nR1 | Work\n";
    assert_eq!(
        input(None, &mut short),
        Ok(b"RUNNING\nR1 | Work\n".to_vec())
    );
}

#[test]
fn a_grammar_refusal_names_the_line_or_the_whole_input() {
    let line = ParseError {
        line: 3,
        why: "the id is used twice",
    };
    assert_eq!(grammar_refusal(&line), "input line 3: the id is used twice");
    let whole = ParseError {
        line: 0,
        why: "the input is empty",
    };
    assert_eq!(grammar_refusal(&whole), "input: the input is empty");
}
