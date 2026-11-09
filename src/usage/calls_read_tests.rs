//! #288 driver regressions of the persisted reader (not frozen spec). Each
//! oracle is a fixture fact (a file length, a hand count, a fresh scan of the
//! same tree, a reader of the same file without the repeat) or a stated
//! invariant, never the reader's own count.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned driver fixture IO and assertions"
)]

use super::Reader;
use crate::tool_stats::{Adapter, Caps};
use crate::tool_stats_fixture::{self as f, Fixture};

fn settle(reader: &mut Reader) {
    reader.round();
    for _ in 0..20_000 {
        if reader.step().is_none() {
            assert!(!reader.busy());
            return;
        }
    }
    panic!("finite fixture reader did not settle");
}

fn read(bytes: &[u8]) -> (Fixture, Reader, std::path::PathBuf) {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    let path = root.claude("store-a", f::C, bytes);
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    settle(&mut reader);
    (root, reader, path)
}

/// The distinct observations the reader holds for `(adapter, id)`.
fn observed(reader: &Reader, adapter: Adapter, id: &str) -> usize {
    reader
        .convs
        .get(&(adapter, id.to_owned()))
        .map_or(0, |held| held.conv.summary.observations)
}

/// A record repeated in one read, and the same record appended again in a
/// later round, is held once: the conversation equals that of the file that
/// holds it once.
#[test]
fn b3_a_repeated_record_is_retained_once() {
    let call = f::call("first", "first", "Read", "model-a", "{}");
    let (_once_root, once, _) = read(call.as_bytes());
    let held = |reader: &Reader| observed(reader, Adapter::Claude, f::C);
    assert!(held(&once) > 0, "the call is read");
    assert_eq!(attempts(&tools(&once)), 1);

    let doubled = format!("{call}{call}");
    let (_root, mut reader, path) = read(doubled.as_bytes());
    assert_eq!(held(&reader), held(&once), "a repeat in one read");
    assert_eq!(attempts(&tools(&reader)), 1);

    let tripled = format!("{doubled}{call}");
    Fixture::rewrite(&path, tripled.as_bytes());
    settle(&mut reader);
    assert_eq!(held(&reader), held(&once), "a repeat appended later");
    assert_eq!(attempts(&tools(&reader)), 1);
}

// --- B3 review r1 regressions (driver; not frozen spec) ---------------------
//
// Each oracle is a fixture fact (a file length, a hand count, a fresh scan of
// the same tree) or a stated invariant, never the reader's own count.

use super::{LSTATS, STEP, Unit, UnitKind, scan};
use crate::stats::Cell;
use crate::tool_stats::{Ask, Filter, GapKind, Page, Scope, View};
use std::fs;

const U: &str = "33333333-3333-4333-8333-333333333333";
const E: &str = "44444444-4444-4444-8444-444444444444";

fn ask(scope: Scope) -> Ask {
    Ask {
        page: Page::Tools,
        scope,
        filter: Filter::All,
    }
}

/// One round to its end, each unit inside the frozen per-unit bounds.
fn drain(reader: &mut Reader) -> Vec<Unit> {
    reader.round();
    let mut units = Vec::new();
    for _ in 0..20_000 {
        let Some(unit) = reader.step() else {
            return units;
        };
        assert!(unit.bytes <= STEP, "actual door bytes: {unit:?}");
        assert!(unit.lstats <= LSTATS, "actual stat calls: {unit:?}");
        assert!(unit.listings <= 1, "one listing per unit: {unit:?}");
        units.push(unit);
    }
    panic!("finite fixture reader did not settle");
}

/// Step the round until a unit of `kind` completed.
fn until(reader: &mut Reader, kind: UnitKind) {
    reader.round();
    for _ in 0..20_000 {
        let unit = reader.step().expect("the round reaches the unit");
        if unit.kind == kind {
            return;
        }
    }
    panic!("no {kind:?} unit");
}

fn tools(reader: &Reader) -> View {
    reader.view(&ask(Scope::All))
}

fn gap(view: &View, kind: &GapKind) -> u64 {
    view.gaps
        .iter()
        .filter(|line| line.kind == *kind)
        .map(|line| line.count)
        .sum()
}

/// A path-free coverage line: one not naming a conversation.
fn global(view: &View, kind: &GapKind) -> bool {
    view.gaps
        .iter()
        .any(|line| line.kind == *kind && line.conversation.is_none() && line.count > 0)
}

fn equals_fresh(root: &Fixture, cached: &Reader) {
    let fresh = scan(root.0.clone(), Some(root.home()), Caps::SLICE);
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        let asked = Ask {
            page,
            ..ask(Scope::All)
        };
        assert_eq!(
            cached.view(&asked),
            fresh.view(&asked),
            "cached equals fresh {page:?}"
        );
    }
}

fn calls(count: usize) -> String {
    (0..count)
        .map(|n| f::call(&format!("c{n}"), &format!("m{n}"), "Read", "model-a", "{}"))
        .collect()
}

#[test]
fn b3_3_prefix_edit_between_chunks_is_changed() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    let pad = format!(
        "{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}\n",
        "x".repeat(8000)
    );
    let original = calls(1) + &pad.repeat(600);
    assert!(
        u64::try_from(original.len()).expect("length") > STEP,
        "two chunks at least"
    );
    let path = root.claude("store-a", f::C, original.as_bytes());
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    until(&mut reader, UnitKind::Read);
    let changed = original.replacen("Read", "Bash", 1);
    assert_eq!(changed.len(), original.len());
    Fixture::rewrite(&path, changed.as_bytes());
    drain(&mut reader);
    assert!(
        gap(&tools(&reader), &GapKind::Changed) > 0,
        "changed under the pass"
    );
    drain(&mut reader);
    equals_fresh(&root, &reader);
    let rows = format!("{:?}", tools(&reader).rows);
    assert!(rows.contains("Bash") && !rows.contains("Read"));
}

/// B3-3: an edit after the prefix verified and before the read resumes is a
/// changed file this round, and the next round equals a fresh scan.
#[test]
fn b3_3_edit_between_verify_and_read_is_changed() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    let first = calls(1);
    let path = root.claude("store-a", f::C, first.as_bytes());
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reader);
    let appended = first + &f::call("later", "later", "Glob", "model-a", "{}");
    Fixture::rewrite(&path, appended.as_bytes());
    until(&mut reader, UnitKind::Verify);
    let changed = appended.replacen("Read", "Bash", 1);
    assert_eq!(changed.len(), appended.len());
    Fixture::rewrite(&path, changed.as_bytes());
    drain(&mut reader);
    assert!(
        gap(&tools(&reader), &GapKind::Changed) > 0,
        "changed after verify"
    );
    drain(&mut reader);
    equals_fresh(&root, &reader);
    let rows = format!("{:?}", tools(&reader).rows);
    assert!(rows.contains("Bash") && !rows.contains("Read"));
}

fn labeled() -> (Fixture, Reader) {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    root.claude("store-a", f::C, calls(1).as_bytes());
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reader);
    let view = tools(&reader);
    assert_eq!(
        view.detail[0].client,
        Cell::Value("work".into()),
        "proven baseline"
    );
    (root, reader)
}

fn unknown(reader: &Reader) {
    let view = tools(reader);
    assert!(!view.detail.is_empty());
    assert!(view.detail.iter().all(|row| row.client == Cell::Unknown));
    assert!(
        global(&view, &GapKind::Unreadable),
        "a path-free coverage line"
    );
}

/// B3-4: a missing journal could have held a retire or reseat: no client is
/// proven from the rest.
#[test]
fn b3_4_missing_journal_is_unavailable_evidence() {
    let (root, mut reader) = labeled();
    fs::remove_file(root.0.join("sessions/alpha/events.jsonl")).expect("drop journal");
    drain(&mut reader);
    unknown(&reader);
}

/// B3-4: an unreadable second session may hold another membership of the
/// same conversation.
#[test]
fn b3_4_unreadable_session_hides_a_membership() {
    let (root, mut reader) = labeled();
    fs::create_dir_all(root.0.join("sessions/beta/meta")).expect("unreadable meta node");
    drain(&mut reader);
    unknown(&reader);
}

/// B3-4: an unreadable archive listing is a coverage gap, never an empty
/// history.
#[test]
fn b3_4_unreadable_archive_listing_is_a_gap() {
    let (root, mut reader) = labeled();
    fs::remove_dir(root.0.join("archive")).expect("drop archive root");
    fs::write(root.0.join("archive"), b"").expect("archive root not a directory");
    drain(&mut reader);
    unknown(&reader);
}

/// B3-5: an invalid recorded binary is normalized before retention; no raw
/// text reaches the reader, the view, its Debug or a rendered line.
#[test]
fn b3_5_invalid_unsupported_binary_is_never_retained() {
    let root = Fixture::new();
    let binary = format!("bad@{}", f::PRIVATE);
    let meta = root.meta("alpha", f::S, &binary, f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reader);
    let view = tools(&reader);
    assert_eq!(gap(&view, &GapKind::Unsupported("unresolved".into())), 1);
    assert!(!format!("{view:?}").contains(f::PRIVATE));
    assert!(!format!("{reader:?}").contains(f::PRIVATE));
    assert!(!format!("{:?}", reader.unsupported).contains(f::PRIVATE));
    for line in &view.gaps {
        assert!(!line.line().contains(f::PRIVATE));
    }
}

/// B3-6: one unsupported conversation counts once however many memberships
/// list it, and only in the scopes its memberships reach.
#[test]
fn b3_6_unsupported_counts_per_conversation_and_scope() {
    let root = Fixture::new();
    for (name, session) in [("alpha", f::S), ("beta", f::T)] {
        let meta = root.meta(name, session, "grok", f::C, Some("work"), "store-a");
        root.live(name, &meta, "");
    }
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reader);
    let grok = |reader: &Reader, scope: Scope| {
        gap(
            &reader.view(&ask(scope)),
            &GapKind::Unsupported("grok".into()),
        )
    };
    assert_eq!(
        grok(&reader, Scope::All),
        1,
        "two memberships, one conversation"
    );
    assert_eq!(grok(&reader, Scope::This(Some(f::S.into()))), 1);
    assert_eq!(grok(&reader, Scope::This(Some(f::T.into()))), 1);
    let gamma = root.meta("gamma", U, "grok", f::D, Some("work"), "store-a");
    root.live("gamma", &gamma, "");
    drain(&mut reader);
    assert_eq!(grok(&reader, Scope::All), 2);
    assert_eq!(
        grok(&reader, Scope::This(Some(f::S.into()))),
        1,
        "not gamma's"
    );
    assert_eq!(grok(&reader, Scope::This(Some(U.into()))), 1, "not alpha's");
    assert_eq!(grok(&reader, Scope::This(None)), 0);
}

/// B3-2 failure receipt: a missing sessions root is one status and no
/// listing.
#[test]
fn b3_2_missing_sessions_root_lists_nothing() {
    let root = Fixture::new();
    fs::remove_dir(root.0.join("sessions")).expect("drop sessions root");
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    let units = drain(&mut reader);
    let sessions = units
        .iter()
        .find(|unit| unit.kind != UnitKind::Load)
        .expect("the sessions unit runs after the load");
    assert_eq!(sessions.kind, UnitKind::Listing);
    assert_eq!(sessions.listings, 0, "no listing happened");
    assert!(sessions.lstats >= 1, "the status did happen");
}

/// B3-2 success and failure receipts of a meta unit: the bytes are the file's
/// length; a meta that is no regular file opens nothing and reads nothing.
#[test]
fn b3_2_meta_receipts_are_the_door_calls() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    let dir = root.live("alpha", &meta, "");
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    let units = drain(&mut reader);
    let read = units
        .iter()
        .find(|unit| unit.kind == UnitKind::Meta)
        .expect("a meta unit");
    let len = fs::metadata(dir.join("meta")).expect("meta length").len();
    assert_eq!((read.bytes, read.opens), (len, 1));
    fs::remove_file(dir.join("meta")).expect("drop meta");
    fs::create_dir(dir.join("meta")).expect("meta not a regular file");
    let units = drain(&mut reader);
    let failed = units
        .iter()
        .find(|unit| unit.kind == UnitKind::Meta)
        .expect("a meta unit");
    assert_eq!((failed.bytes, failed.opens), (0, 0));
    assert!(failed.lstats >= 1);
}

// --- B3 review r2 regressions (driver; measured RED on r2, lead ruling
// ae-20261108T144928Z-d9d1c43b) ------------------------------------------------

use std::collections::BTreeSet;

fn attempts(view: &View) -> u64 {
    match view.totals {
        crate::tool_stats::Cols::Tool(cols) => cols.attempts,
        other => panic!("Tools: {other:?}"),
    }
}

fn codex_one_call() -> String {
    f::codex_call("x", None, "read") + &f::codex_result("x")
}

/// One more seat line set for slot `slot` of an existing fixture meta.
fn seat(slot: &str, binary: &str, home: &std::path::Path, id: &str) -> String {
    format!(
        "seat.{slot}={slot}\nprofile.{slot}=fixture-profile\nagent_bin.{slot}={binary}\nconfig_home.{slot}={}\nharness_session.{slot}={id}\n",
        home.display()
    )
}

/// The round's receipts without the bytes (a meta's length depends on its
/// recorded spelling).
fn calls_made(units: &[Unit]) -> (u64, u64, u64) {
    units
        .iter()
        .fold((0, 0, 0), |(stats, listings, opens), unit| {
            (
                stats + u64::from(unit.lstats),
                listings + u64::from(unit.listings),
                opens + u64::from(unit.opens),
            )
        })
}

/// B3-8: a Codex find over a day whose listing failed does no IO at all:
/// a regular file where a day directory belongs refuses for every user.
#[test]
fn b3_8_codex_find_does_no_retry_io_for_a_failed_day() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "codex", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    let rollout = root.codex("store-a", f::C, codex_one_call().as_bytes());
    let sessions = root.store("store-a").join("sessions");
    let fixture_day = rollout.parent().expect("day directory").to_owned();
    let other = crate::quota::codex_rollout_dirs(&sessions, f::C)
        .expect("a UUIDv7 id names its days")
        .into_iter()
        .find(|dir| *dir != fixture_day)
        .expect("a neighbour day");
    fs::create_dir_all(other.parent().expect("month directory")).expect("month");
    fs::write(&other, b"").expect("a file where a day directory belongs");
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    let units = drain(&mut reader);
    let find = units.last().expect("the find is the round's last unit");
    assert_eq!(find.kind, UnitKind::Lstats);
    assert_eq!(
        (find.lstats, find.listings, find.opens, find.bytes),
        (0, 0, 0, 0),
        "no retry IO"
    );
    assert!(gap(&tools(&reader), &GapKind::Unreadable) > 0);
}

/// B3-2 ruling (ii): a recorded home spelled through a link is another
/// copy of the same store; the fold union merges it, counts do not change.
#[test]
fn b3_2_link_spelling_is_an_extra_copy_with_unchanged_counts() {
    let root = Fixture::new();
    let link = root.0.join("link-a");
    std::os::unix::fs::symlink(root.store("store-a"), &link).expect("store link");
    let meta = root.meta("alpha", f::S, "codex", f::C, Some("work"), "store-a")
        + &seat("w1", "codex", &link, f::C);
    root.live("alpha", &meta, "");
    root.codex("store-a", f::C, codex_one_call().as_bytes());
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reader);
    let copies: BTreeSet<[u8; 16]> = reader.convs[&(Adapter::Codex, f::C.to_owned())]
        .conv
        .summary
        .files
        .keys()
        .map(|stream| stream.copy)
        .collect();
    assert_eq!(copies.len(), 2, "two spellings, two copies: {copies:?}");
    let view = tools(&reader);
    assert_eq!(attempts(&view), 1, "the union merges the copies");
    assert_eq!(gap(&view, &GapKind::Conflict), 0);
}

/// A deep store holding one Claude call, recorded either directly or through
/// a 20-link chain from a shallow name.
fn deep_store(chain: bool) -> (Fixture, Vec<Unit>, u64) {
    let root = Fixture::new();
    let store = "deep/a/b/c/d/e";
    let recorded = if chain {
        let mut target = root.store(store);
        for link in (0..20).rev() {
            let name = root.0.join(format!("l{link}"));
            std::os::unix::fs::symlink(&target, &name).expect("chain link");
            target = name;
        }
        "l0"
    } else {
        store
    };
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), recorded);
    root.live("alpha", &meta, "");
    root.claude(store, f::C, calls(1).as_bytes());
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    let units = drain(&mut reader);
    let count = attempts(&tools(&reader));
    (root, units, count)
}

/// B3-2 ruling (iii): a long link chain costs one receipt per syscall, the
/// same calls as the direct spelling, inside the unit bound.
#[test]
fn b3_2_link_chain_costs_one_receipt_per_syscall() {
    let (_direct_root, direct, direct_held) = deep_store(false);
    let (_chain_root, chained, chain_held) = deep_store(true);
    assert_eq!((direct_held, chain_held), (1, 1), "both spellings locate");
    assert_eq!(
        calls_made(&chained),
        calls_made(&direct),
        "the chain adds no receipt"
    );
}

/// B3-2 ruling (iv): no path-depth limit: a 30-component store locates.
#[test]
fn b3_2_deep_recorded_store_is_located() {
    let root = Fixture::new();
    let base = root.0.components().count();
    assert!(base < 28, "the fixture root is shallow enough");
    let store = (base..30)
        .map(|level| format!("d{level}"))
        .collect::<Vec<_>>()
        .join("/");
    assert_eq!(root.store(&store).components().count(), 30);
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), &store);
    root.live("alpha", &meta, "");
    root.claude(&store, f::C, calls(1).as_bytes());
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reader);
    assert_eq!(attempts(&tools(&reader)), 1);
    assert_eq!(gap(&tools(&reader), &GapKind::Unlocated), 0);
}

/// B3-2: seats naming missing deep homes add no store-resolution IO beyond
/// one status of each store.
#[test]
fn b3_2_many_seats_add_no_resolution_io() {
    let run = |extra: bool| {
        let root = Fixture::new();
        let mut meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
        if extra {
            for (slot, id) in [("w1", f::D), ("w2", U), ("w3", E)] {
                let home = root
                    .store("store-a")
                    .join(slot)
                    .join("n1/n2/n3/n4/n5/n6/n7/n8");
                meta.push_str(&seat(slot, "claude", &home, id));
            }
        }
        root.live("alpha", &meta, "");
        root.claude("store-a", f::C, calls(1).as_bytes());
        let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
        calls_made(&drain(&mut reader)).0
    };
    let (one, four) = (run(false), run(true));
    assert!(four <= one + 3 * 2, "three missing stores: {one} -> {four}");
}

// --- P3 Q1 = A (lead ruling ae-20261109T145706Z-e8d89b84) --------------------

/// A path-shaped sentinel the fixture root does not carry.
const HOSTILE: &str = "HOSTILE_PATH_SENTINEL";

/// Run the real S5 maintainer until it proves an epoch.
fn prove_epoch(root: &Fixture) {
    let mut maintainer = crate::stats_store::maint::Maintainer::new(
        &root.0,
        crate::stats_store::disk::LIMITS,
        std::time::Duration::from_millis(100),
    );
    for _ in 0..100 {
        let _ = maintainer.step();
        if root.0.join("stats/epoch").is_file() {
            return;
        }
    }
    panic!("the S5 maintainer proves an epoch");
}

fn stats_files(directory: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(directory).expect("owned stats directory") {
        let path = entry.expect("stats entry").path();
        if path.is_dir() {
            stats_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Q1 = A: a transcript whose cwd, tool-argument paths and skill base
/// directory carry a sentinel leaves no sentinel byte under `stats/`, and
/// the association cache holds exactly the roots the session meta recorded.
#[test]
fn q1_transcript_paths_never_persist_and_only_recorded_roots_do() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    prove_epoch(&root);
    let hostile = format!("/{HOSTILE}/project");
    let bytes = format!(
        "{{\"type\":\"user\",\"cwd\":\"{hostile}\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"hi\"}}]}}}}\n"
    ) + &f::call(
        "skill-id",
        "skill-message",
        "Skill",
        "model-a",
        "{\"skill\":\"allowed-skill\"}",
    ) + &f::result("skill-id", false, Some(true))
        + &f::base("skill-id", &format!("{hostile}/skills/allowed-skill"))
        + &f::call(
            "read-id",
            "read-message",
            "Read",
            "model-a",
            &format!("{{\"file_path\":\"{hostile}/skills/allowed-skill/a.md\"}}"),
        )
        + &f::result("read-id", false, None);
    root.claude("store-a", f::C, bytes.as_bytes());
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reader);
    assert_eq!(
        attempts(&tools(&reader)),
        2,
        "positive control: both calls read"
    );

    let mut paths = Vec::new();
    stats_files(&root.0.join("stats"), &mut paths);
    let tools_dir = root.0.join("stats/tools");
    assert!(
        paths
            .iter()
            .any(|path| path.starts_with(tools_dir.join("conv"))),
        "positive control: a conversation document is persisted"
    );
    for path in &paths {
        let bytes = fs::read(path).expect("persisted bytes");
        assert!(
            !bytes
                .windows(HOSTILE.len())
                .any(|part| part == HOSTILE.as_bytes()),
            "a transcript path persisted in {:?}",
            path.strip_prefix(&root.0)
        );
    }

    let sources = crate::tool_stats::doc::parse_sources(
        &fs::read(tools_dir.join("sources")).expect("the association cache"),
    )
    .expect("the cache parses");
    crate::tool_stats::doc::parse_assoc(
        &fs::read(tools_dir.join("assoc")).expect("the association document"),
    )
    .expect("the association document parses");
    let mut roots = BTreeSet::new();
    for record in sources.values() {
        for seat in &record.seats {
            roots.extend(seat.store.iter().cloned());
            roots.extend(seat.config_homes.iter().cloned());
            roots.extend(seat.work_dirs.iter().cloned());
        }
    }
    let recorded: BTreeSet<String> = [root.store("store-a"), root.0.join("work")]
        .iter()
        .map(|path| path.to_str().expect("utf-8 fixture root").to_owned())
        .collect();
    assert_eq!(roots, recorded, "exactly the roots the meta recorded");
}

// --- P3 review r1 regressions (driver; review-20261109T153353Z-f8d89344) ----
//
// Each oracle is a fixture fact, a cold scan of the same tree, or a stated
// invariant (no rollback, the cap holds, a document is not replaced).

use crate::stats_store::disk;
use crate::tool_stats::doc;
use crate::tool_stats::incr;
use crate::tool_stats::store::{self, Loaded};

/// The Tools attempts of the view document published under `root`.
fn published_attempts(root: &Fixture) -> u64 {
    let mut files = disk::Files::at(&root.0);
    let Loaded::Ok(view) = store::load_view(&mut files) else {
        panic!("a published view");
    };
    attempts(&incr::combine(&view, &ask(Scope::All), &Caps::SLICE))
}

/// P3-1: a writer that loses the generation comparison never publishes its
/// older counts past the winner, in that round or the next.
#[test]
fn p3_1_stale_writer_cannot_roll_back_the_view() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    prove_epoch(&root);
    let one = calls(1);
    let path = root.claude("store-a", f::C, one.as_bytes());
    let mut winner = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut winner);
    let mut loser = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut loser);
    assert_eq!(published_attempts(&root), 1, "positive control");

    let two = one + &f::call("second", "second", "Glob", "model-a", "{}");
    Fixture::rewrite(&path, two.as_bytes());
    loser.round();
    for _ in 0..20_000 {
        let front = loser.round.as_ref().and_then(|round| round.tasks.front());
        if matches!(front, Some(super::Task::Publish(_))) {
            break;
        }
        assert!(loser.step().is_some(), "the loser reaches its publication");
    }
    let three = two + &f::call("third", "third", "Bash", "model-a", "{}");
    Fixture::rewrite(&path, three.as_bytes());
    drain(&mut winner);
    assert_eq!(published_attempts(&root), 3, "the winner published");
    drain(&mut loser);
    assert_eq!(
        published_attempts(&root),
        3,
        "the loser's round rolled back"
    );
    drain(&mut loser);
    assert_eq!(published_attempts(&root), 3, "the loser's next round");
    let cold = scan(root.0.clone(), Some(root.home()), Caps::SLICE);
    assert_eq!(attempts(&tools(&cold)), 3, "the fixture holds three calls");
}

/// P3-2: a native child file name never reaches `stats/`, and the child's
/// calls still count as their own stream.
#[test]
fn p3_2_child_file_names_never_persist() {
    const CHILD: &str = "CHILD_STEM_SENTINEL";
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    prove_epoch(&root);
    root.claude("store-a", f::C, calls(1).as_bytes());
    root.child(
        "store-a",
        f::C,
        &format!("agent-{CHILD}"),
        f::call("child", "child", "Glob", "model-a", "{}").as_bytes(),
    );
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reader);
    assert_eq!(attempts(&tools(&reader)), 2, "positive control: both files");
    let mut paths = Vec::new();
    stats_files(&root.0.join("stats"), &mut paths);
    assert!(
        paths
            .iter()
            .any(|path| path.starts_with(root.0.join("stats/tools/conv"))),
        "positive control: a conversation document is persisted"
    );
    for path in &paths {
        let bytes = fs::read(path).expect("persisted bytes");
        assert!(
            !bytes
                .windows(CHILD.len())
                .any(|part| part == CHILD.as_bytes()),
            "a child file name persisted in {:?}",
            path.strip_prefix(&root.0)
        );
    }
    let mut reopened = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    let units = drain(&mut reopened);
    assert!(
        units
            .iter()
            .all(|unit| unit.kind != UnitKind::Read || unit.bytes == 0),
        "the persisted child identity resumes: {units:?}"
    );
    assert_eq!(attempts(&tools(&reopened)), 2);
}

/// P3-3: a recorded work dir edit reclassifies an unchanged transcript as a
/// cold scan does, warm and reopened; a client-only relabel reads no byte.
#[test]
fn p3_3_recorded_root_edit_rebuilds_and_client_relabel_reads_nothing() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    let session = root.live("alpha", &meta, "");
    prove_epoch(&root);
    let other = root.0.join("other");
    let skill = other.join(".claude/skills/demo/SKILL.md");
    let bytes = f::call(
        "read",
        "read",
        "Read",
        "model-a",
        &format!("{{\"file_path\":\"{}\"}}", skill.display()),
    ) + &f::result("read", false, None);
    root.claude("store-a", f::C, bytes.as_bytes());
    let mut warm = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut warm);
    equals_fresh(&root, &warm);
    let skills = Ask {
        page: Page::Skills,
        ..ask(Scope::All)
    };
    let before = scan(root.0.clone(), Some(root.home()), Caps::SLICE).view(&skills);

    let work = root.0.join("work").display().to_string();
    let moved = meta.replace(&work, &other.display().to_string());
    assert_ne!(moved, meta);
    Fixture::rewrite(&session.join("meta"), moved.as_bytes());
    let after = scan(root.0.clone(), Some(root.home()), Caps::SLICE).view(&skills);
    assert_ne!(before, after, "positive control: the root classifies");
    drain(&mut warm);
    equals_fresh(&root, &warm);
    let mut reopened = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reopened);
    equals_fresh(&root, &reopened);

    let relabeled = moved.replace("client.main=work", "client.main=home");
    assert_ne!(relabeled, moved);
    Fixture::rewrite(&session.join("meta"), relabeled.as_bytes());
    let units = drain(&mut warm);
    assert!(
        units
            .iter()
            .all(|unit| !matches!(unit.kind, UnitKind::Read | UnitKind::Verify)),
        "a client relabel reads no transcript byte: {units:?}"
    );
    equals_fresh(&root, &warm);
}

/// P3-4: a journal prefix rewritten between two chunks of one scan is not
/// trusted under its new status: the next round equals a cold scan.
#[test]
fn p3_4_journal_rewrite_between_chunks_is_not_trusted() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    let retire = |id: &str| {
        let summary = format!(
            "tool=claude profile=fixture-profile config_home={} config_home_base=",
            root.store("store-a").display()
        );
        format!(
            "{{\"ts\":\"2026-11-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"old-worker\",\"target_slot\":\"spawned.0\",\"ref\":\"{id}\",\"summary\":\"{summary}\"}}\n"
        )
    };
    let pad = format!("{}\n", " ".repeat(1000)).repeat(5000);
    let original = retire(U) + &pad;
    assert!(
        u64::try_from(original.len()).expect("length") > STEP,
        "two chunks at least"
    );
    let session = root.live("alpha", &meta, &original);
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    until(&mut reader, UnitKind::Journal);
    until(&mut reader, UnitKind::Journal);
    let rewritten = retire(E) + &pad;
    assert_eq!(rewritten.len(), original.len());
    Fixture::rewrite(&session.join("events.jsonl"), rewritten.as_bytes());
    drain(&mut reader);
    drain(&mut reader);
    let cold = scan(root.0.clone(), Some(root.home()), Caps::SLICE);
    assert!(
        cold.conversations().iter().any(|one| one.id == E),
        "positive control: the rewritten record associates"
    );
    assert_eq!(reader.conversations(), cold.conversations());
}

/// P3-5: an unreadable association cache is never replaced: the IO refusal
/// stands and nothing is renamed over it.
#[test]
fn p3_5_unreadable_sources_are_not_replaced() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    prove_epoch(&root);
    root.claude("store-a", f::C, calls(1).as_bytes());
    let mut first = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut first);
    let sources = root.0.join("stats/tools/sources");
    assert!(
        sources.is_file(),
        "positive control: the cache is published"
    );
    let beta = root.meta("beta", U, "claude", E, Some("home"), "store-a");
    root.live("beta", &beta, "");
    let before = fs::metadata(&sources).expect("cache status").ino();
    fs::set_permissions(&sources, fs::Permissions::from_mode(0o000)).expect("deny reads");
    let mut second = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut second);
    let status = fs::metadata(&sources).expect("cache status");
    let (ino, mode) = (status.ino(), status.mode() & 0o777);
    fs::set_permissions(&sources, fs::Permissions::from_mode(0o600)).expect("restore");
    assert_eq!(
        (ino, mode),
        (before, 0),
        "the unreadable cache was replaced"
    );
}

/// Run the real S5 maintainer until the active epoch is proven again.
fn repair_epoch(root: &Fixture) -> u64 {
    let mut maintainer = crate::stats_store::maint::Maintainer::new(
        &root.0,
        disk::LIMITS,
        std::time::Duration::from_millis(100),
    );
    for _ in 0..200 {
        let _ = maintainer.step();
        if let Ok(Some(epoch)) = disk::active_epoch(&mut disk::Files::at(&root.0)) {
            return epoch;
        }
    }
    panic!("the S5 maintainer proves an epoch again");
}

/// P3-6: an epoch proof lost and then proven again by an S5 repair never
/// lets held counts of the old payload into the new one.
#[test]
fn p3_6_lost_epoch_proof_never_resurrects_old_counts() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    prove_epoch(&root);
    let old = disk::active_epoch(&mut disk::Files::at(&root.0)).expect("a proven epoch");
    let path = root.claude("store-a", f::C, calls(1).as_bytes());
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reader);
    fs::remove_file(&path).expect("the source goes");
    drain(&mut reader);
    assert_eq!(
        attempts(&tools(&reader)),
        1,
        "positive control: P6 keeps it"
    );
    fs::write(root.0.join("stats/epoch"), b"damaged\n").expect("damage the proof");
    assert!(disk::active_epoch(&mut disk::Files::at(&root.0)).is_err());
    drain(&mut reader);
    let new = repair_epoch(&root);
    assert_ne!(Some(new), old, "positive control: a new payload");
    drain(&mut reader);
    assert_eq!(attempts(&tools(&reader)), 0, "old counts resurrected");
    let cold = scan(root.0.clone(), Some(root.home()), Caps::SLICE);
    assert_eq!(attempts(&tools(&cold)), 0);
}

/// P3-7: the published prefix fits the exact view cap at the generation it
/// is published with, across a decimal-width boundary.
#[test]
fn p3_7_view_fits_its_cap_at_the_actual_generation() {
    let conv = incr::publish(&incr::Conv::new(Adapter::Claude, f::C).summary, None);
    let size = |generation: u64| {
        doc::render_view(&incr::Published {
            generation,
            convs: vec![conv.clone()],
            over_cap: 0,
        })
        .len()
    };
    assert_eq!(size(10), size(9) + 1, "positive control: one more digit");
    let mut caps = Caps::SLICE;
    caps.view_cap = u64::try_from(size(9)).expect("small");
    let mut reader = Reader::with(std::path::PathBuf::from("/nonexistent"), None, caps, false);
    reader.generation = 8;
    reader
        .enter(&(Adapter::Claude, f::C.to_owned()), Some(conv.clone()))
        .expect("admitted");
    assert_eq!(
        doc::render_view(&reader.capped(9)).len(),
        size(9),
        "fits at 9"
    );
    let ten = doc::render_view(&reader.capped(10));
    assert!(
        u64::try_from(ten.len()).expect("small") <= caps.view_cap,
        "over the cap by {} at 10",
        ten.len() - size(9)
    );
}

/// P3-3 r2 (lead ae-20261109T160844Z-8b530340, no P6 amendment): a root edit
/// reads the present streams again under the new roots, and a gone stream
/// keeps its counts under the classification it was read with, until `r`.
#[test]
fn p3_3_root_edit_keeps_gone_stream_counts_under_their_classification() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    let session = root.live("alpha", &meta, "");
    prove_epoch(&root);
    let read = |id: &str, path: &std::path::Path| {
        f::call(
            id,
            id,
            "Read",
            "model-a",
            &format!("{{\"file_path\":\"{}\"}}", path.display()),
        ) + &f::result(id, false, None)
    };
    let work = root.0.join("work");
    let other = root.0.join("other");
    root.claude(
        "store-a",
        f::C,
        read("parent", &other.join(".claude/skills/demo/SKILL.md")).as_bytes(),
    );
    let child = root.child(
        "store-a",
        f::C,
        "agent-a",
        read("child", &work.join(".claude/skills/old/SKILL.md")).as_bytes(),
    );
    let skills = Ask {
        page: Page::Skills,
        ..ask(Scope::All)
    };
    let reads = |view: &View| match view.totals {
        crate::tool_stats::Cols::Skill(cols) => cols.reads,
        other => panic!("Skills: {other:?}"),
    };
    let mut warm = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut warm);
    assert_eq!(reads(&warm.view(&skills)), 1, "positive control: old only");
    fs::remove_file(&child).expect("the child goes");
    drain(&mut warm);
    assert_eq!(reads(&warm.view(&skills)), 1, "P6 keeps the gone child");

    let moved = meta.replace(&work.display().to_string(), &other.display().to_string());
    assert_ne!(moved, meta);
    Fixture::rewrite(&session.join("meta"), moved.as_bytes());
    drain(&mut warm);
    let view = warm.view(&skills);
    let rows = format!("{:?}", view.rows);
    assert!(
        rows.contains("\"demo\""),
        "the present stream reclassified: {rows}"
    );
    assert!(
        rows.contains("\"old\""),
        "the gone stream kept its count: {rows}"
    );
    assert_eq!(reads(&view), 2);
    assert!(gap(&warm.view(&ask(Scope::All)), &GapKind::SourceGone) > 0);
    let mut reopened = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reopened);
    assert_eq!(reopened.view(&skills), view, "the reopened store agrees");
}

// --- P3 review r3 regressions (driver; review-20261109T162852Z-478953be) ----

use std::collections::BTreeMap;

/// Every way a counted conversation is rebuilt from its sources.
#[derive(Clone, Copy, Debug)]
enum Trigger {
    /// The parent is another file (a new inode).
    IdentityChange,
    /// The parent is cut below what was consumed of it.
    Truncation,
    /// The parent keeps its length, but its consumed bytes differ.
    WitnessMismatch,
    /// The parent grows, but its consumed bytes differ.
    AppendConflict,
    /// The recorded roots change: every event is classified again.
    RootEdit,
}

const TRIGGERS: [Trigger; 5] = [
    Trigger::IdentityChange,
    Trigger::Truncation,
    Trigger::WitnessMismatch,
    Trigger::AppendConflict,
    Trigger::RootEdit,
];

fn named_calls(prefix: &str, tool: &str, count: usize) -> String {
    (0..count)
        .map(|n| {
            let id = format!("{prefix}{n}");
            f::call(&id, &id, tool, "model-a", "{}")
        })
        .collect()
}

/// Each tool's attempts on the Tools page.
fn tool_attempts(view: &View) -> BTreeMap<String, u64> {
    view.rows
        .iter()
        .filter_map(|row| match (&row.item, row.cols) {
            (
                crate::tool_stats::Item::Tool(crate::tool_stats::Ident::Name(name)),
                crate::tool_stats::Cols::Tool(cols),
            ) => Some((name.clone(), cols.attempts)),
            _ => None,
        })
        .collect()
}

/// A parent with three `Read` calls and a child with one `Grep` call.
struct Family {
    root: Fixture,
    meta: String,
    session: std::path::PathBuf,
    parent: std::path::PathBuf,
    child: std::path::PathBuf,
}

impl Family {
    fn new() -> Self {
        let root = Fixture::new();
        let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
        let session = root.live("alpha", &meta, "");
        prove_epoch(&root);
        let parent = root.claude("store-a", f::C, named_calls("p", "Read", 3).as_bytes());
        let child = root.child(
            "store-a",
            f::C,
            "agent-a",
            named_calls("k", "Grep", 1).as_bytes(),
        );
        Self {
            root,
            meta,
            session,
            parent,
            child,
        }
    }

    fn reader(&self) -> Reader {
        Reader::new(self.root.0.clone(), Some(self.root.home()), Caps::SLICE)
    }

    /// Apply `trigger` and return the parent's counts it must then show.
    fn trigger(&self, trigger: Trigger) -> BTreeMap<String, u64> {
        let old = named_calls("p", "Read", 3);
        let counts = |pairs: &[(&str, u64)]| {
            pairs
                .iter()
                .map(|(tool, count)| ((*tool).to_owned(), *count))
                .collect::<BTreeMap<_, _>>()
        };
        match trigger {
            Trigger::IdentityChange => {
                fs::remove_file(&self.parent).expect("the old parent goes");
                Fixture::rewrite(&self.parent, named_calls("p", "Glob", 2).as_bytes());
                counts(&[("Glob", 2)])
            }
            Trigger::Truncation => {
                let short = named_calls("p", "Glob", 1);
                assert!(short.len() < old.len(), "positive control: shorter");
                Fixture::rewrite(&self.parent, short.as_bytes());
                counts(&[("Glob", 1)])
            }
            Trigger::WitnessMismatch => {
                let same = old.replace("Read", "Bash");
                assert_eq!(same.len(), old.len(), "positive control: same length");
                Fixture::rewrite(&self.parent, same.as_bytes());
                counts(&[("Bash", 3)])
            }
            Trigger::AppendConflict => {
                let grown = old.replace("Read", "Bash") + &named_calls("w", "Write", 1);
                Fixture::rewrite(&self.parent, grown.as_bytes());
                counts(&[("Bash", 3), ("Write", 1)])
            }
            Trigger::RootEdit => {
                let work = self.root.0.join("work").display().to_string();
                let other = self.root.0.join("other").display().to_string();
                let moved = self.meta.replace(&work, &other);
                assert_ne!(moved, self.meta, "positive control: the roots move");
                Fixture::rewrite(&self.session.join("meta"), moved.as_bytes());
                counts(&[("Read", 3)])
            }
        }
    }
}

/// Step the round until every file is listed and none is checked yet: the
/// locate phase has no task left, so the next step queues the lstats and
/// runs the first batch.
fn until_lstats(reader: &mut Reader) {
    reader.round();
    for _ in 0..20_000 {
        let listed = reader.round.as_ref().is_some_and(|round| {
            matches!(round.phase, super::Phase::Locate) && round.tasks.is_empty()
        });
        if listed {
            return;
        }
        reader.step().expect("the round reaches its lstats");
    }
    panic!("no lstats unit");
}

/// The counted view with the gone child kept and `parent` from its source.
fn kept(parent: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    let mut want = parent.clone();
    *want.entry("Grep".to_owned()).or_default() += 1;
    want
}

/// R3-2 (lead ae-20261109T160844Z-8b530340 and the r3 ruling): P6 holds
/// across every rebuild. A counted child that is gone keeps its count when
/// the present parent is then rebuilt, by every trigger, through a durable
/// reopen; and the rebuilt parent shows only its new source.
#[test]
fn r3_2_every_rebuild_trigger_keeps_a_gone_streams_counts() {
    every_row(gone_then_rebuilt);
}

/// Run `row` for every trigger; every failing row is named, not only the
/// first.
fn every_row(row: fn(Trigger)) {
    let failed: Vec<Trigger> = TRIGGERS
        .into_iter()
        .filter(|trigger| std::panic::catch_unwind(|| row(*trigger)).is_err())
        .collect();
    assert!(failed.is_empty(), "failing rows: {failed:?}");
}

fn gone_then_rebuilt(trigger: Trigger) {
    {
        let family = Family::new();
        let mut warm = family.reader();
        drain(&mut warm);
        let tools_page = ask(Scope::All);
        assert_eq!(
            tool_attempts(&warm.view(&tools_page)),
            kept(&BTreeMap::from([("Read".to_owned(), 3)])),
            "{trigger:?}: positive control"
        );
        fs::remove_file(&family.child).expect("the child goes");
        drain(&mut warm);
        assert!(
            gap(&warm.view(&tools_page), &GapKind::SourceGone) > 0,
            "{trigger:?}"
        );
        let parent = family.trigger(trigger);
        drain(&mut warm);
        let view = warm.view(&tools_page);
        assert_eq!(tool_attempts(&view), kept(&parent), "{trigger:?}: rebuilt");
        assert!(
            gap(&view, &GapKind::SourceGone) > 0,
            "{trigger:?}: still gone"
        );
        let mut reopened = family.reader();
        drain(&mut reopened);
        assert_eq!(
            tool_attempts(&reopened.view(&tools_page)),
            kept(&parent),
            "{trigger:?}: the reopened store agrees"
        );
    }
}

/// R3-2: a child listed, then deleted before its own lstat, in the round
/// that rebuilds its conversation, is classified gone by that lstat and
/// keeps its counts: nothing clears it first.
#[test]
fn r3_2_a_child_deleted_between_listing_and_its_lstat_keeps_its_counts() {
    every_row(deleted_after_listing);
}

fn deleted_after_listing(trigger: Trigger) {
    {
        let family = Family::new();
        let mut warm = family.reader();
        drain(&mut warm);
        let tools_page = ask(Scope::All);
        let parent = family.trigger(trigger);
        until_lstats(&mut warm);
        fs::remove_file(&family.child).expect("the child goes after listing");
        drain(&mut warm);
        let view = warm.view(&tools_page);
        assert_eq!(tool_attempts(&view), kept(&parent), "{trigger:?}: rebuilt");
        assert!(gap(&view, &GapKind::SourceGone) > 0, "{trigger:?}: gone");
        let mut reopened = family.reader();
        drain(&mut reopened);
        assert_eq!(
            tool_attempts(&reopened.view(&tools_page)),
            kept(&parent),
            "{trigger:?}: the reopened store agrees"
        );
    }
}

/// R3-2, the damage row: a damaged conversation document is the one
/// rebuild with no evidence left to carry, because the document is the
/// only durable copy of a gone stream's counts. The present parent is
/// rebuilt from its source alone (no stale or doubled count), and the
/// reopened store agrees. The gone child's count is lost with the damaged
/// bytes. This boundary is reported to lead; it is not P6 by design.
#[test]
fn r3_2_a_damaged_document_rebuilds_the_present_sources_only() {
    let family = Family::new();
    let mut warm = family.reader();
    drain(&mut warm);
    fs::remove_file(&family.child).expect("the child goes");
    drain(&mut warm);
    drop(warm);
    let tools_page = ask(Scope::All);
    let name = store::conv_name(Adapter::Claude, f::C);
    let path = family.root.0.join(name);
    assert!(path.exists(), "positive control: {}", path.display());
    Fixture::rewrite(&path, b"not a conversation\n");
    let mut reopened = family.reader();
    drain(&mut reopened);
    let parent = BTreeMap::from([("Read".to_owned(), 3)]);
    assert_eq!(tool_attempts(&reopened.view(&tools_page)), parent);
    let mut again = family.reader();
    drain(&mut again);
    assert_eq!(tool_attempts(&again.view(&tools_page)), parent);
}

// --- Lead ruling B2 (ae-20261109T165907Z-00e19e67): visible damage loss ----

/// The damage mark's line on the Tools page, if any.
fn damage_lines(view: &View) -> Vec<String> {
    view.gaps
        .iter()
        .filter(|line| line.kind == GapKind::RebuiltFromSource)
        .map(crate::tool_stats::GapLine::line)
        .collect()
}

/// B2: a conversation rebuilt from a damaged document carries a sticky,
/// visible mark in its own document: it survives every later source
/// rebuild and a reopen, a second damage sets it again (once), and only
/// `r` clears it. A conversation never damaged has none.
#[test]
fn b2_a_damaged_document_marks_its_conversation_until_reset() {
    let family = Family::new();
    let tools_page = ask(Scope::All);
    let mut warm = family.reader();
    drain(&mut warm);
    fs::remove_file(&family.child).expect("the child goes");
    drain(&mut warm);
    assert_eq!(
        damage_lines(&warm.view(&tools_page)),
        Vec::<String>::new(),
        "control: never damaged, no mark"
    );
    drop(warm);
    let path = family.root.0.join(store::conv_name(Adapter::Claude, f::C));
    let one = vec![
        "rebuilt from source: 1 conversations (counts of deleted sources there are lost)"
            .to_owned(),
    ];
    Fixture::rewrite(&path, b"not a conversation\n");
    let mut reopened = family.reader();
    drain(&mut reopened);
    assert_eq!(damage_lines(&reopened.view(&tools_page)), one, "marked");
    // A later source rebuild keeps the mark: it is not a damage.
    family.trigger(Trigger::IdentityChange);
    drain(&mut reopened);
    assert_eq!(damage_lines(&reopened.view(&tools_page)), one, "sticky");
    drop(reopened);
    let mut again = family.reader();
    drain(&mut again);
    assert_eq!(damage_lines(&again.view(&tools_page)), one, "durable");
    drop(again);
    // Damage of the marked document loses the mark with it; the new damage
    // sets it again on the rebuilt conversation, so the line stays true.
    Fixture::rewrite(&path, b"damaged again\n");
    let mut twice = family.reader();
    drain(&mut twice);
    assert_eq!(
        damage_lines(&twice.view(&tools_page)),
        one,
        "set again, once"
    );
    drop(twice);
    assert!(!matches!(
        disk::reset(&family.root.0, std::time::Duration::ZERO).expect("statistics reset"),
        disk::Reset::Busy
    ));
    let mut fresh = family.reader();
    drain(&mut fresh);
    assert_eq!(
        damage_lines(&fresh.view(&tools_page)),
        Vec::<String>::new(),
        "r clears the mark"
    );
}

/// R3-2: the conversation-wide rebuild every other trigger takes (a count
/// that cannot be added up, a view that cannot) keeps every stream's
/// evidence and its source mark, and forgets only the file states: each
/// file is checked by its own lstat and read again from 0.
#[test]
fn r3_2_a_conversation_rebuild_keeps_evidence_and_forgets_file_states() {
    let family = Family::new();
    let mut warm = family.reader();
    drain(&mut warm);
    fs::remove_file(&family.child).expect("the child goes");
    drain(&mut warm);
    let key = (Adapter::Claude, f::C.to_owned());
    let held = warm
        .convs
        .get(&key)
        .expect("the conversation is held")
        .conv
        .clone();
    let streams: Vec<_> = held.summary.files.keys().cloned().collect();
    assert_eq!(streams.len(), 2, "positive control: two files");
    assert!(
        streams
            .iter()
            .all(|stream| held.resume.file(stream).is_some()),
        "positive control: both files have a state"
    );
    let mut rebuilt = held.clone();
    rebuilt.rebuild();
    // The summary as its held evidence makes it: a document round trip
    // derives it again from the events, so a kept summary over cleared
    // events does not pass.
    let derived = doc::parse_conv(&doc::render_conv(&rebuilt)).expect("canonical");
    assert_eq!(
        derived.summary, held.summary,
        "every count and source mark kept"
    );
    assert!(
        streams
            .iter()
            .all(|stream| rebuilt.resume.file(stream).is_none()),
        "every file is read again from 0"
    );
}

// --- P3 review r2 regressions (driver; review-20261109T160806Z-734d0fe3) ----

fn coverage_size(coverage: &incr::Coverage) -> u64 {
    u64::try_from(doc::render_coverage(coverage).len()).expect("small document")
}

/// R2-2: produced coverage fits its exact cap: whole at its own size, the
/// longest prefix below it, the rest counted in a visible over-cap line.
#[test]
fn r2_2_coverage_fits_its_exact_cap_with_a_visible_omission() {
    let coverage = incr::Coverage {
        generation: 9,
        unsupported: (0..5)
            .map(|n| ("grok".to_owned(), BTreeSet::from([Some(format!("s{n}"))])))
            .collect(),
        ..incr::Coverage::default()
    };
    let whole = coverage_size(&coverage);
    assert_eq!(doc::fit_coverage(&coverage, whole), coverage, "fits whole");
    let three = incr::Coverage {
        unsupported: coverage.unsupported[..3].to_vec(),
        omitted: 2,
        ..coverage.clone()
    };
    assert_eq!(
        doc::fit_coverage(&coverage, coverage_size(&three)),
        three,
        "the exact boundary keeps three"
    );
    let fitted = doc::fit_coverage(&coverage, whole - 1);
    assert!(coverage_size(&fitted) < whole, "under the cap");
    assert!(fitted.omitted > 0);
    assert_eq!(
        u64::try_from(fitted.unsupported.len()).expect("small") + fitted.omitted,
        5,
        "every conversation is listed or counted"
    );
    assert_eq!(
        doc::parse_coverage(&doc::render_coverage(&fitted)),
        Ok(fitted.clone())
    );
    let mut view = tools_of_empty(9);
    fitted.apply(&mut view, &ask(Scope::All));
    let lines: Vec<String> = view
        .gaps
        .iter()
        .map(crate::tool_stats::GapLine::line)
        .collect();
    assert!(
        lines.contains(&format!("over cap, not counted {}", fitted.omitted)),
        "{lines:?}"
    );
}

/// `coverage` keeping its first `kept` records, the rest counted as
/// omitted: the candidate a fitter may return.
fn kept_prefix(coverage: &incr::Coverage, kept: usize) -> incr::Coverage {
    let left = coverage.unsupported.len() - kept;
    incr::Coverage {
        unsupported: coverage.unsupported[..kept].to_vec(),
        omitted: coverage.omitted + u64::try_from(left).expect("small"),
        ..coverage.clone()
    }
}

fn equal_records(count: usize) -> incr::Coverage {
    incr::Coverage {
        generation: 9,
        unsupported: (0..count)
            .map(|_| ("grok".to_owned(), BTreeSet::from([Some("s".to_owned())])))
            .collect(),
        ..incr::Coverage::default()
    }
}

/// R3-1: the document trailer counts the kept records, so it grows a digit
/// at 9 -> 10, 99 -> 100 and 999 -> 1000 kept. The review's counterexample:
/// 99 equal records, E the empty document with omitted = 99, L one record's
/// cost. At cap E + 9L, keeping nine renders end=10, one byte past the cap.
#[test]
fn r3_1_coverage_fit_accounts_for_trailer_digit_growth() {
    let coverage = equal_records(99);
    let empty = coverage_size(&kept_prefix(&coverage, 0));
    let one = coverage_size(&kept_prefix(&coverage, 1)) - empty;
    let cap = empty + 9 * one;
    let fitted = doc::fit_coverage(&coverage, cap);
    assert!(
        coverage_size(&fitted) <= cap,
        "kept {} renders {} over cap {cap}",
        fitted.unsupported.len(),
        coverage_size(&fitted)
    );
    assert_eq!(
        fitted,
        kept_prefix(&coverage, 8),
        "the longest fitting prefix"
    );
    // Every trailer and omitted-digit boundary: at the exact size of the
    // k-prefix the fitter keeps exactly k, one byte less keeps exactly k - 1.
    for count in [10, 11, 99, 100, 101, 1_000, 1_001] {
        let coverage = equal_records(count);
        for kept in [0, 1, 8, 9, 10, 11, 98, 99, 100, 101, 998, 999, 1_000] {
            if kept >= count {
                continue;
            }
            let cap = coverage_size(&kept_prefix(&coverage, kept));
            let fitted = doc::fit_coverage(&coverage, cap);
            assert_eq!(
                fitted,
                kept_prefix(&coverage, kept),
                "{count} records, cap of {kept}"
            );
            if kept > 0 {
                let fitted = doc::fit_coverage(&coverage, cap - 1);
                assert!(coverage_size(&fitted) < cap, "{count} records under {cap}");
                assert_eq!(
                    fitted,
                    kept_prefix(&coverage, kept - 1),
                    "{count} records, one byte under the cap of {kept}"
                );
            }
        }
    }
}

/// The Tools page of an empty published view of `generation`.
fn tools_of_empty(generation: u64) -> View {
    incr::combine(
        &incr::Published {
            generation,
            convs: Vec::new(),
            over_cap: 0,
        },
        &ask(Scope::All),
        &Caps::SLICE,
    )
}

/// R2-2: a coverage document over its cap is never written: the paired
/// publication refuses, so nothing unreadable is ever marked published.
#[test]
fn r2_2_store_refuses_coverage_over_its_cap() {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        "",
    );
    prove_epoch(&root);
    let session = "s".repeat(200);
    let huge = incr::Coverage {
        generation: 1,
        unsupported: (0..30_000)
            .map(|n| {
                (
                    format!("tool{n:05}"),
                    BTreeSet::from([Some(session.clone())]),
                )
            })
            .collect(),
        ..incr::Coverage::default()
    };
    assert!(coverage_size(&huge) > doc::COVERAGE_CAP, "positive control");
    let view = incr::Published {
        generation: 1,
        convs: Vec::new(),
        over_cap: 0,
    };
    let planned = disk::tools(&root.0, std::time::Duration::from_secs(1), |dir, _| {
        (None, store::publish_view_with(dir, &view, &huge).is_ok())
    });
    assert!(
        matches!(planned, disk::Tools::Done { value: false, .. }),
        "an over-cap coverage publication is refused"
    );
}

/// R2-2: a real reader's coverage over a small cap is published readable,
/// with its over-cap line, and a durable reopen draws the same lines.
#[test]
fn r2_2_reader_coverage_over_cap_publishes_a_readable_line() {
    let root = Fixture::new();
    let home = root.store("store-a");
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a")
        + &seat("g1", "grok", &home, U)
        + &seat("g2", "grok", &home, E)
        + &seat("g3", "grok", &home, f::D);
    root.live("alpha", &meta, "");
    prove_epoch(&root);
    root.claude("store-a", f::C, calls(1).as_bytes());
    let caps = Caps {
        coverage_cap: 160,
        ..Caps::SLICE
    };
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), caps);
    drain(&mut reader);
    let mut files = disk::Files::at(&root.0);
    let Loaded::Ok(coverage) = store::load_coverage(&mut files) else {
        panic!("a readable coverage document");
    };
    let Loaded::Ok(view) = store::load_view(&mut files) else {
        panic!("a published view");
    };
    assert_eq!(coverage.generation, view.generation, "one generation");
    assert!(coverage_size(&coverage) <= 160, "under the cap");
    assert!(coverage.omitted > 0, "positive control: the cap bites");
    assert_eq!(
        u64::try_from(coverage.unsupported.len()).expect("small") + coverage.omitted,
        3
    );
    let mut drawn = incr::combine(&view, &ask(Scope::All), &Caps::SLICE);
    coverage.apply(&mut drawn, &ask(Scope::All));
    let line = format!("over cap, not counted {}", coverage.omitted);
    assert!(
        drawn.gaps.iter().any(|gap| gap.line() == line),
        "{:?}",
        drawn.gaps
    );
    assert_eq!(
        drawn,
        reader.view(&ask(Scope::All)),
        "the reader draws the same"
    );
    let mut reopened = Reader::new(root.0.clone(), Some(root.home()), caps);
    drain(&mut reopened);
    assert_eq!(
        reopened.view(&ask(Scope::All)),
        drawn,
        "the reopened reader"
    );
}

/// R2-3: an unreadable coverage document is never replaced; a damaged one is
/// rebuilt by the next publication.
#[test]
fn r2_3_unreadable_coverage_is_not_replaced_and_damaged_is_rebuilt() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        "",
    );
    prove_epoch(&root);
    let mut held = calls(1);
    let path = root.claude("store-a", f::C, held.as_bytes());
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain(&mut reader);
    let coverage = root.0.join("stats/tools/coverage");
    assert!(
        coverage.is_file(),
        "positive control: coverage is published"
    );
    let before = fs::metadata(&coverage).expect("coverage status").ino();
    fs::set_permissions(&coverage, fs::Permissions::from_mode(0o000)).expect("deny reads");
    held += &f::call("more", "more", "Glob", "model-a", "{}");
    Fixture::rewrite(&path, held.as_bytes());
    drain(&mut reader);
    let status = fs::metadata(&coverage).expect("coverage status");
    let (ino, mode) = (status.ino(), status.mode() & 0o777);
    fs::set_permissions(&coverage, fs::Permissions::from_mode(0o600)).expect("restore");
    assert_eq!(
        (ino, mode),
        (before, 0),
        "the unreadable coverage was replaced"
    );

    fs::write(&coverage, b"damaged\n").expect("damage the document");
    held += &f::call("last", "last", "Bash", "model-a", "{}");
    Fixture::rewrite(&path, held.as_bytes());
    drain(&mut reader);
    assert!(
        matches!(
            store::load_coverage(&mut disk::Files::at(&root.0)),
            Loaded::Ok(_)
        ),
        "a damaged document is rebuilt"
    );
    assert_eq!(published_attempts(&root), 3);
}

// --- P3 review r4 regression (driver; review-20261109T165800Z-2807a8cf) ----

/// R4-1: replacing every file of a conversation at its exact cap is
/// admitted as the conversation it becomes, never as a mixture of new and
/// superseded evidence. With `per_conversation` = 4: parent 2 + child 2
/// attempts, both replaced, parent 3 + child 1. Taking the parent first
/// against the old child saw 5 > 4 and refused the conversation for good,
/// although the final four fit, as a cold rebuild shows.
#[test]
fn r4_1_replacing_every_file_at_the_cap_admits_the_final_conversation() {
    let caps = Caps {
        per_conversation: 4,
        ..Caps::SLICE
    };
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    prove_epoch(&root);
    let parent = root.claude("store-a", f::C, named_calls("p", "Bash", 2).as_bytes());
    let child = root.child(
        "store-a",
        f::C,
        "agent-a",
        named_calls("k", "Grep", 2).as_bytes(),
    );
    let tools_page = ask(Scope::All);
    let mut warm = Reader::new(root.0.clone(), Some(root.home()), caps);
    drain(&mut warm);
    let before = warm.view(&tools_page);
    assert_eq!(
        tool_attempts(&before),
        BTreeMap::from([("Bash".to_owned(), 2), ("Grep".to_owned(), 2)]),
        "positive control: exactly at the cap, admitted"
    );
    assert_eq!(gap(&before, &GapKind::Ledger), 0);
    for (path, bytes) in [
        (&parent, named_calls("q", "Glob", 3)),
        (&child, named_calls("j", "Write", 1)),
    ] {
        fs::remove_file(path).expect("the old file goes");
        Fixture::rewrite(path, bytes.as_bytes());
    }
    drain(&mut warm);
    let want = BTreeMap::from([("Glob".to_owned(), 3), ("Write".to_owned(), 1)]);
    let view = warm.view(&tools_page);
    assert_eq!(
        gap(&view, &GapKind::Ledger),
        0,
        "not refused: {:?}",
        view.gaps
    );
    assert_eq!(tool_attempts(&view), want, "the final four");
    let cold = scan(root.0.clone(), Some(root.home()), caps);
    assert_eq!(view, cold.view(&tools_page), "warm == cold");
    let mut reopened = Reader::new(root.0.clone(), Some(root.home()), caps);
    drain(&mut reopened);
    assert_eq!(
        reopened.view(&tools_page),
        view,
        "the reopened store agrees"
    );
}
