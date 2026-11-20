//! Lazy `Resume` acceptance: plan r4 section 1 and lead ruling 2026-11-10.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned synthetic fixture IO and independent acceptance assertions"
)]

use super::{LSTATS, Reader, STEP, Unit, UnitKind, scan, take_units};
use crate::stats_store::disk;
use crate::tool_stats::store::{self, Loaded};
use crate::tool_stats::{Adapter, Ask, Caps, Cols, Filter, GapKind, Page, Scope};
use crate::tool_stats_fixture::{self as f, Fixture};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

struct Case {
    root: Fixture,
    parent: PathBuf,
    child: PathBuf,
    codex: PathBuf,
    held: Reader,
}

struct Receipts {
    units: Vec<Unit>,
    reads: Vec<(String, u64)>,
}

fn ask(page: Page) -> Ask {
    Ask {
        page,
        scope: Scope::All,
        filter: Filter::All,
    }
}

fn open(root: &Fixture) -> Reader {
    Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE)
}

fn round(reader: &mut Reader) -> Receipts {
    let _ = take_units();
    let _ = disk::take_payload_names();
    reader.round();
    let mut returned = Vec::new();
    for _ in 0..30_000 {
        let Some(unit) = reader.step() else {
            assert!(!reader.busy(), "the finite fixture round has ended");
            let units = take_units();
            assert_eq!(
                units, returned,
                "the actual door observer covers every unit"
            );
            return Receipts {
                units,
                reads: disk::take_payload_names(),
            };
        };
        assert!(unit.bytes <= STEP, "native byte bound: {unit:?}");
        assert!(unit.lstats <= LSTATS, "lstat bound: {unit:?}");
        returned.push(unit);
    }
    panic!("the finite lazy-resume fixture did not settle");
}

fn prove_epoch(root: &Fixture) {
    let mut maintainer = crate::stats_store::maint::Maintainer::new(
        &root.0,
        disk::LIMITS,
        Duration::from_millis(100),
    );
    for _ in 0..100 {
        let _ = maintainer.step();
        if root.0.join("stats/epoch").is_file() {
            return;
        }
    }
    panic!("the real S5 first publication must establish the epoch");
}

fn call(id: &str, tool: &str) -> String {
    f::call(id, id, tool, "model-a", "{}") + &f::result(id, false, None)
}

fn fixture() -> Case {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        &f::event("ordinary", "a real S5 epoch source"),
    );
    root.live(
        "beta",
        &root.meta("beta", f::T, "codex", f::D, Some("home"), "store-b"),
        "",
    );
    let parent = root.claude("store-a", f::C, call("parent-one", "Read").as_bytes());
    let child = root.child(
        "store-a",
        f::C,
        "agent-one",
        call("child-one", "Write").as_bytes(),
    );
    let text = f::context("turn-one", "model-b", "high")
        + &f::codex_call("codex-one", Some("turn-one"), "exec")
        + &f::codex_result("codex-one");
    let codex = root.codex("store-b", f::D, text.as_bytes());
    prove_epoch(&root);
    let mut held = open(&root);
    round(&mut held);
    counts(&held, 3);
    for (adapter, id) in [(Adapter::Claude, f::C), (Adapter::Codex, f::D)] {
        assert!(
            size(&root.0.join(store::conv_name(adapter, id))) > 0,
            "control: both authoritative conversation documents exist"
        );
    }
    Case {
        root,
        parent,
        child,
        codex,
        held,
    }
}

fn size(path: &Path) -> u64 {
    fs::metadata(path).expect("fixture file metadata").len()
}

fn counts(reader: &Reader, attempts: u64) {
    let view = reader.view(&ask(Page::Tools));
    let Cols::Tool(cols) = view.totals else {
        panic!("Tools columns");
    };
    assert_eq!(
        (cols.attempts, cols.returned),
        (attempts, attempts),
        "hand-counted complete calls"
    );
    assert_eq!(view.listed, 2, "one Claude and one Codex conversation");
}

fn same(left: &Reader, right: &Reader) {
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        assert_eq!(
            left.view(&ask(page)),
            right.view(&ask(page)),
            "whole {page:?} view"
        );
    }
}

fn native(receipts: &Receipts) -> u64 {
    receipts
        .units
        .iter()
        .filter(|unit| matches!(unit.kind, UnitKind::Read | UnitKind::Verify))
        .map(|unit| unit.bytes)
        .sum()
}

fn conversation_reads(receipts: &Receipts) -> BTreeMap<String, u64> {
    let mut reads = BTreeMap::new();
    for (name, bytes) in &receipts.reads {
        if name.starts_with("stats/tools/conv/") && *bytes != 0 {
            *reads.entry(name.clone()).or_default() += bytes;
        }
    }
    reads
}

fn only_loaded(receipts: &Receipts, expected: &BTreeMap<String, u64>) {
    assert_eq!(
        &conversation_reads(receipts),
        expected,
        "only changed conversations may load their authoritative Summary + Resume document"
    );
    let loaded: u64 = receipts
        .units
        .iter()
        .filter(|unit| unit.kind == UnitKind::Load)
        .map(|unit| unit.store_bytes)
        .sum();
    let expected_conv_bytes: u64 = expected.values().sum();
    assert!(
        loaded > expected_conv_bytes,
        "control: Load receipts also include global documents"
    );
}

fn changed_document(case: &Case, adapter: Adapter, id: &str) -> BTreeMap<String, u64> {
    let name = store::conv_name(adapter, id);
    let bytes = size(&case.root.0.join(&name));
    BTreeMap::from([(name, bytes)])
}

fn no_publish(receipts: &Receipts) {
    assert_eq!(
        receipts
            .units
            .iter()
            .filter(|unit| unit.kind == UnitKind::Publish)
            .count(),
        0,
        "unchanged data queues no Publish unit, including zero-byte units"
    );
}

#[test]
fn lazy_warm_open_loads_zero_conversation_document_bytes() {
    let case = fixture();
    let mut reopened = open(&case.root);
    let receipts = round(&mut reopened);
    only_loaded(&receipts, &BTreeMap::new());
    assert_eq!(native(&receipts), 0, "unchanged native transcripts");
    assert_eq!(
        receipts.units.iter().map(|unit| unit.bytes).sum::<u64>(),
        0,
        "unchanged meta and journals too"
    );
    counts(&reopened, 3);
    same(&reopened, &case.held);
}

#[test]
fn lazy_warm_open_with_no_changes_runs_zero_publish_units() {
    let case = fixture();
    let before = snapshot(&case.root.0.join("stats/tools"));
    let mut reopened = open(&case.root);
    let receipts = round(&mut reopened);
    no_publish(&receipts);
    assert_eq!(
        snapshot(&case.root.0.join("stats/tools")),
        before,
        "no durable byte changes"
    );
    counts(&reopened, 3);
    same(&reopened, &case.held);
}

#[test]
fn lazy_unlocated_warm_open_loads_zero_conversation_bytes_and_publishes_nothing() {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        &f::event("ordinary", "a real S5 epoch source"),
    );
    // No transcript is created: this recorded conversation is Unlocated.
    prove_epoch(&root);
    let mut held = open(&root);
    round(&mut held);
    let before = held.view(&ask(Page::Tools));
    assert_eq!((before.read, before.listed), (0, 1));
    let Cols::Tool(cols) = &before.totals else {
        panic!("Tools columns");
    };
    assert_eq!((cols.attempts, cols.returned), (0, 0), "no native calls");
    assert_eq!(before.gaps.len(), 1, "only the missing-history gap");
    assert_eq!(before.gaps[0].kind, GapKind::Unlocated);
    assert_eq!(before.gaps[0].count, 1, "one unlocated conversation");
    assert!(
        size(&root.0.join(store::conv_name(Adapter::Claude, f::C))) > 0,
        "control: a positive-size authoritative conversation document exists"
    );
    let saved = snapshot(&root.0.join("stats/tools"));
    drop(held);

    let mut reopened = open(&root);
    let receipts = round(&mut reopened);
    assert_eq!(
        reopened.view(&ask(Page::Tools)),
        before,
        "an unchanged reopen preserves the gap and all counts"
    );
    assert_eq!(native(&receipts), 0, "there is no native history to read");
    no_publish(&receipts);
    assert_eq!(
        snapshot(&root.0.join("stats/tools")),
        saved,
        "the durable documents are unchanged"
    );
    only_loaded(&receipts, &BTreeMap::new());
}

#[test]
fn lazy_append_loads_only_its_conversation_and_matches_cold() {
    let case = fixture();
    let expected = changed_document(&case, Adapter::Claude, f::C);
    let appended = call("parent-two", "Edit");
    OpenOptions::new()
        .append(true)
        .open(&case.parent)
        .expect("existing parent")
        .write_all(appended.as_bytes())
        .expect("actual append");
    let mut reopened = open(&case.root);
    let receipts = round(&mut reopened);
    counts(&reopened, 4);
    let cold = scan(case.root.0.clone(), Some(case.root.home()), Caps::SLICE);
    same(&reopened, &cold);
    only_loaded(&receipts, &expected);
    let gained = u64::try_from(appended.len()).expect("small fixture append");
    assert!(
        native(&receipts) >= gained && native(&receipts) <= gained + (1 << 20),
        "append plus at most one witness block, never the other conversation"
    );
    let mut again = open(&case.root);
    let steady = round(&mut again);
    only_loaded(&steady, &BTreeMap::new());
    no_publish(&steady);
    same(&again, &reopened);
}

#[test]
fn lazy_source_gone_loads_only_its_conversation_and_keeps_counts() {
    let case = fixture();
    let mut reference = fixture();
    let expected = changed_document(&case, Adapter::Claude, f::C);
    fs::remove_file(&case.child).expect("one child goes");
    fs::remove_file(&reference.child).expect("the same reference child goes");
    round(&mut reference.held);
    let mut reopened = open(&case.root);
    let receipts = round(&mut reopened);
    counts(&reopened, 3);
    same(&reopened, &reference.held);
    assert_eq!(
        native(&receipts),
        0,
        "gone is a state change, not a transcript read"
    );
    let view = reopened.view(&ask(Page::Tools));
    assert_eq!(
        view.gaps
            .iter()
            .filter(|gap| gap.kind == GapKind::SourceGone)
            .map(|gap| gap.count)
            .sum::<u64>(),
        1,
        "one deleted source retains its counts"
    );
    only_loaded(&receipts, &expected);
}

#[test]
fn lazy_recorded_root_change_loads_only_its_conversation_and_matches_cold() {
    let case = fixture();
    let expected = changed_document(&case, Adapter::Claude, f::C);
    let mut meta = case
        .root
        .meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    writeln!(
        meta,
        "work_dir.main={}",
        case.root.0.join("changed-work").display()
    )
    .expect("fixture metadata line");
    // Replace the existing field rather than introduce a duplicate key.
    let old = format!("work_dir.main={}\n", case.root.0.join("work").display());
    meta = meta.replace(&old, "");
    Fixture::rewrite(&case.root.0.join("sessions/alpha/meta"), meta.as_bytes());
    let mut reopened = open(&case.root);
    let receipts = round(&mut reopened);
    counts(&reopened, 3);
    let cold = scan(case.root.0.clone(), Some(case.root.home()), Caps::SLICE);
    same(&reopened, &cold);
    only_loaded(&receipts, &expected);
    assert_eq!(
        native(&receipts),
        size(&case.parent) + size(&case.child),
        "root reclassification rereads exactly the Claude conversation"
    );
}

#[test]
fn lazy_discovery_mark_change_loads_only_its_conversation() {
    let case = fixture();
    let mut reference = fixture();
    let expected = changed_document(&case, Adapter::Codex, f::D);
    for item in [&case, &reference] {
        let twin = item
            .codex
            .with_file_name(format!("rollout-twin-{}.jsonl", f::D));
        fs::copy(&item.codex, twin).expect("a second locator match, not a new observation");
    }
    round(&mut reference.held);
    let mut reopened = open(&case.root);
    let receipts = round(&mut reopened);
    same(&reopened, &reference.held);
    let view = reopened.view(&ask(Page::Tools));
    assert_eq!(
        view.gaps
            .iter()
            .filter(|gap| gap.kind == GapKind::NotUnique)
            .map(|gap| gap.count)
            .sum::<u64>(),
        1,
        "one ambiguous conversation"
    );
    assert_eq!(native(&receipts), 0, "ambiguous files are not read");
    only_loaded(&receipts, &expected);
}

// Snapshot and restore only our owned Tools subtree. This builds real plan
// crash prefixes without planting a cursor, a summary or an epoch.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(dir).expect("owned Tools subtree") {
        let path = entry.expect("owned entry").path();
        if path.is_dir() {
            for (name, bytes) in snapshot(&path) {
                result.insert(
                    PathBuf::from(path.file_name().expect("directory name")).join(name),
                    bytes,
                );
            }
        } else {
            result.insert(
                PathBuf::from(path.file_name().expect("file name")),
                fs::read(path).expect("owned file"),
            );
        }
    }
    result
}

fn restore(dir: &Path, saved: &BTreeMap<PathBuf, Vec<u8>>) {
    fs::remove_dir_all(dir).expect("owned previous prefix");
    fs::create_dir(dir).expect("owned clean Tools subtree");
    for (name, bytes) in saved {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("owned parent")).expect("owned subdirectory");
        fs::write(path, bytes).expect("whole previous fixture document");
    }
}

#[test]
fn lazy_crash_prefix_recovers_a_view_behind_its_conversation() {
    let mut case = fixture();
    let directory = case.root.0.join("stats/tools");
    let old = snapshot(&directory);
    let appended = call("parent-after-crash", "Edit");
    OpenOptions::new()
        .append(true)
        .open(&case.parent)
        .expect("existing parent")
        .write_all(appended.as_bytes())
        .expect("append before interrupted publication");
    round(&mut case.held);
    counts(&case.held, 4);
    let mut files = disk::Files::at(&case.root.0);
    let Loaded::Ok(conv) = store::load_conv(&mut files, Adapter::Claude, f::C) else {
        panic!("real updated conversation");
    };
    let Loaded::Ok(view) = store::load_view(&mut files) else {
        panic!("real updated view");
    };
    let Loaded::Ok(coverage) = store::load_coverage(&mut files) else {
        panic!("real paired coverage");
    };
    restore(&directory, &old);
    let plan = store::publish_with(&mut files, &conv, &view, &coverage)
        .expect("real paired publication plan");
    assert!(
        plan.ops
            .iter()
            .any(|op| matches!(op, disk::Op::Rename { to, .. }
        if *to == store::conv_name(Adapter::Claude, f::C))),
        "control: the plan includes its conversation commit"
    );
    for upto in 0..=plan.ops.len() {
        restore(&directory, &old);
        plan.apply(&mut files, upto)
            .expect("actual publication crash prefix");
        let mut recovered = open(&case.root);
        round(&mut recovered);
        counts(&recovered, 4);
        let cold = scan(case.root.0.clone(), Some(case.root.home()), Caps::SLICE);
        same(&recovered, &cold);
        let mut again = open(&case.root);
        let receipts = round(&mut again);
        only_loaded(&receipts, &BTreeMap::new());
        no_publish(&receipts);
        same(&again, &recovered);
    }
}
