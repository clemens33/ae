//! #288 B3 driver regression (B2 review r1 F2 objection 2, the ownership
//! boundary): the reader keeps every retained stream canonical, each
//! distinct event once at exact capacity, because the fold charges only the
//! canonical conversation. Not frozen spec; the oracle is that invariant
//! and a reader of the same file without the repeat, never the reader's own
//! count.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned driver fixture IO and assertions"
)]

use super::Reader;
use crate::tool_stats::{Caps, Conversation};
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

fn assert_canonical(conversations: &[Conversation]) {
    for conversation in conversations {
        for (index, stream) in conversation.streams.iter().enumerate() {
            assert!(
                stream.events.windows(2).all(|pair| pair[0] < pair[1]),
                "each distinct event once"
            );
            assert_eq!(
                stream.events.capacity(),
                stream.events.len(),
                "exact capacity"
            );
            assert!(
                conversation.streams[..index]
                    .iter()
                    .all(|seen| (&seen.role, seen.copy) != (&stream.role, stream.copy)),
                "one stream per (role, copy)"
            );
        }
    }
}

/// A record repeated in one read, and the same record appended again in a
/// later round, is retained once: the streams equal those of the file that
/// holds it once.
#[test]
fn b3_a_repeated_record_is_retained_once() {
    let call = f::call("first", "first", "Read", "model-a", "{}");
    let (_once_root, once, _) = read(call.as_bytes());
    assert_canonical(once.conversations());
    let held = |reader: &Reader| -> usize {
        reader
            .conversations()
            .iter()
            .flat_map(|conversation| &conversation.streams)
            .map(|stream| stream.events.len())
            .sum()
    };
    assert!(held(&once) > 0, "the call is read");

    let doubled = format!("{call}{call}");
    let (_root, mut reader, path) = read(doubled.as_bytes());
    assert_canonical(reader.conversations());
    assert_eq!(held(&reader), held(&once), "a repeat in one read");

    let tripled = format!("{doubled}{call}");
    Fixture::rewrite(&path, tripled.as_bytes());
    settle(&mut reader);
    assert_canonical(reader.conversations());
    assert_eq!(held(&reader), held(&once), "a repeat appended later");
}

/// D1 (lead brief, before B4): one full first build over a sandboxed COPY
/// of a real history. Ignored; run by hand with `AE_D1_STATE` (the copied
/// ae state root) and `AE_D1_HOME` (the copied home). It prints structure
/// only: counts, bytes and time, never a name, path or record.
#[test]
#[ignore = "D1 measurement over a sandboxed history copy, run by hand"]
fn d1_first_build_measure() {
    let state = std::env::var_os("AE_D1_STATE").expect("AE_D1_STATE names the copied state root");
    let home = std::env::var_os("AE_D1_HOME").expect("AE_D1_HOME names the copied home");
    let started = std::time::Instant::now();
    let mut reader = Reader::new(state.into(), Some(home.into()), Caps::SLICE);
    reader.round();
    let (mut units, mut bytes, mut lstats, mut listings, mut opens) =
        (0_u64, 0, 0_u64, 0_u64, 0_u64);
    while let Some(unit) = reader.step() {
        units += 1;
        bytes += unit.bytes;
        lstats += u64::from(unit.lstats);
        listings += u64::from(unit.listings);
        opens += u64::from(unit.opens);
    }
    let read_ms = started.elapsed().as_millis();
    let ask = crate::tool_stats::Ask {
        page: crate::tool_stats::Page::Tools,
        scope: crate::tool_stats::Scope::All,
        filter: crate::tool_stats::Filter::All,
    };
    let view = reader.view(&ask);
    let total_ms = started.elapsed().as_millis();
    let conversations = reader.conversations();
    let streams: usize = conversations.iter().map(|one| one.streams.len()).sum();
    let events: usize = conversations
        .iter()
        .flat_map(|one| &one.streams)
        .map(|stream| stream.events.len())
        .sum();
    let gap = |kind: &crate::tool_stats::GapKind| -> u64 {
        view.gaps
            .iter()
            .filter(|line| line.kind == *kind)
            .map(|line| line.count)
            .sum()
    };
    println!(
        "D1 units={units} bytes={bytes} lstats={lstats} listings={listings} opens={opens} \
         read_ms={read_ms} total_ms={total_ms} conversations={} streams={streams} \
         events={events} event_bytes_floor={} listed={} read={} ledger_refused={} \
         unreadable={} changed={} partial={}",
        conversations.len(),
        events * size_of::<crate::tool_stats::Event>(),
        view.listed,
        view.read,
        gap(&crate::tool_stats::GapKind::Ledger),
        gap(&crate::tool_stats::GapKind::Unreadable),
        gap(&crate::tool_stats::GapKind::Changed),
        view.partial(),
    );
}

// --- B3 review r1 regressions (driver; not frozen spec) ---------------------
//
// Each oracle is a fixture fact (a file length, a hand count, a fresh scan of
// the same tree) or a stated invariant, never the reader's own count.

use super::{LSTATS, STEP, Unit, UnitKind, scan};
use crate::stats::Cell;
use crate::tool_stats::{Ask, Filter, GapKind, Page, Scope, VIEW_RESERVE, View};
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

fn held(reader: &Reader, id: &str) -> usize {
    reader
        .conversations()
        .iter()
        .filter(|conversation| conversation.id == id)
        .flat_map(|conversation| &conversation.streams)
        .map(|stream| stream.events.len())
        .sum()
}

fn tracked(reader: &Reader, id: &str) -> bool {
    reader.files.keys().any(|place| place.conversation.1 == id)
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

/// The events `calls(n)` retains, by hand count of the fixture (lead ruling
/// ae-20261108T151321Z-defcb7d1): each `f::call` line is one assistant record with
/// one `tool_use`, so one `Attempt` (its own id) and one `Stamp`; every
/// fixture call carries the same timestamp `2026-11-01T00:00:01Z`, so the
/// stamps are ONE distinct event: n attempts + 1 stamp.
const fn events_of_calls(n: usize) -> usize {
    n + 1
}

fn calls(count: usize) -> String {
    (0..count)
        .map(|n| f::call(&format!("c{n}"), &format!("m{n}"), "Read", "model-a", "{}"))
        .collect()
}

/// B3-1: a raw cap of only the view reserve retains no observation and no
/// file state, and says so.
#[test]
fn b3_1_view_reserve_cap_retains_nothing() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    root.claude("store-a", f::C, calls(3).as_bytes());
    let caps = Caps {
        ledger: VIEW_RESERVE,
        ..Caps::SLICE
    };
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), caps);
    drain(&mut reader);
    assert_eq!(held(&reader, f::C), 0, "no observation retained");
    assert!(reader.files.is_empty(), "no file state retained");
    assert!(
        global(&tools(&reader), &GapKind::Unreadable),
        "coverage says so"
    );
}

/// B3-1: a conversation over its observation cap is refused alone, its
/// observations and file state evicted; another conversation stays.
#[test]
fn b3_1_oversized_conversation_is_refused_alone() {
    let root = Fixture::new();
    let alpha = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    let beta = root.meta("beta", f::T, "claude", f::D, Some("work"), "store-b");
    root.live("alpha", &alpha, "");
    root.live("beta", &beta, "");
    root.claude("store-a", f::C, calls(5).as_bytes());
    root.claude("store-b", f::D, calls(1).as_bytes());
    let caps = Caps {
        per_conversation: 2,
        ..Caps::SLICE
    };
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), caps);
    drain(&mut reader);
    assert_eq!(held(&reader, f::C), 0, "five calls over a cap of two");
    assert!(!tracked(&reader, f::C), "its file state is evicted");
    assert_eq!(
        held(&reader, f::D),
        events_of_calls(1),
        "the other conversation stays"
    );
    assert!(gap(&tools(&reader), &GapKind::Ledger) > 0);
}

/// B3-1: growth past the cap on a later round evicts what was admitted.
#[test]
fn b3_1_growth_past_the_cap_evicts() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    let path = root.claude("store-a", f::C, calls(1).as_bytes());
    let caps = Caps {
        per_conversation: 2,
        ..Caps::SLICE
    };
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), caps);
    drain(&mut reader);
    assert_eq!(
        held(&reader, f::C),
        events_of_calls(1),
        "admitted while under the cap"
    );
    Fixture::rewrite(&path, calls(5).as_bytes());
    drain(&mut reader);
    assert_eq!(held(&reader, f::C), 0, "grown past the cap");
    assert!(!tracked(&reader, f::C), "its file state is evicted");
    assert!(gap(&tools(&reader), &GapKind::Ledger) > 0);
}

/// B3-3: the consumed prefix edited in place between two chunks of one
/// pass is a changed file this round, and the next round equals a fresh scan.
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
    let sessions = units.first().expect("the sessions unit runs first");
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

fn caps_at(ledger: usize) -> Caps {
    Caps {
        ledger,
        ..Caps::SLICE
    }
}

fn scan_at(root: &Fixture, ledger: usize) -> Reader {
    scan(root.0.clone(), Some(root.home()), caps_at(ledger))
}

/// The least ledger (bytes) at which `admits` holds, by bisection over the
/// fixture's fresh scans; `admits` is monotone in the ledger.
fn smallest_ledger(admits: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (VIEW_RESERVE, Caps::SLICE.ledger);
    assert!(admits(hi), "the slice cap admits the fixture");
    assert!(!admits(lo), "the view reserve alone does not");
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if admits(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

fn equals_fresh_at(root: &Fixture, cached: &Reader, ledger: usize) {
    let fresh = scan_at(root, ledger);
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        let asked = Ask {
            page,
            ..ask(Scope::All)
        };
        assert_eq!(
            cached.view(&asked),
            fresh.view(&asked),
            "cached equals fresh {page:?} at ledger {ledger}"
        );
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
    let copies: BTreeSet<u32> = reader
        .conversations()
        .iter()
        .filter(|conversation| conversation.id == f::C)
        .flat_map(|conversation| &conversation.streams)
        .map(|stream| stream.copy)
        .collect();
    assert_eq!(copies.len(), 2, "two spellings, two copies: {copies:?}");
    let view = tools(&reader);
    assert_eq!(attempts(&view), 1, "the union merges the copies");
    assert_eq!(gap(&view, &GapKind::Conflict), 0);
}

/// A deep store holding one Claude call, recorded either directly or through
/// a 20-link chain from a shallow name.
fn deep_store(chain: bool) -> (Fixture, Vec<Unit>, usize) {
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
    let count = held(&reader, f::C);
    (root, units, count)
}

/// B3-2 ruling (iii): a long link chain costs one receipt per syscall, the
/// same calls as the direct spelling, inside the unit bound.
#[test]
fn b3_2_link_chain_costs_one_receipt_per_syscall() {
    let (_direct_root, direct, direct_held) = deep_store(false);
    let (_chain_root, chained, chain_held) = deep_store(true);
    let one = events_of_calls(1);
    assert_eq!(
        (direct_held, chain_held),
        (one, one),
        "both spellings locate"
    );
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
    assert_eq!(held(&reader, f::C), events_of_calls(1));
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

/// B3-1a: Codex day listings over several days stay inside a small ledger
/// after every unit, beside the observations held.
#[test]
fn b3_1a_day_listings_stay_inside_a_small_ledger_after_every_unit() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "codex", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    root.codex("store-a", f::C, codex_one_call().as_bytes());
    let sessions = root.store("store-a").join("sessions");
    for day in crate::quota::codex_rollout_dirs(&sessions, f::C).expect("days") {
        fs::create_dir_all(&day).expect("day");
        for n in 0..500 {
            fs::write(day.join(format!("{n:04}-{}", "x".repeat(200))), b"").expect("filler");
        }
    }
    let space = 64 << 10;
    let mut reader = Reader::new(
        root.0.clone(),
        Some(root.home()),
        caps_at(VIEW_RESERVE + space),
    );
    reader.round();
    for _ in 0..20_000 {
        let Some(unit) = reader.step() else {
            return;
        };
        assert!(unit.listings <= 1, "{unit:?}");
        let days = reader
            .round
            .as_ref()
            .map_or(0, |round| round.days.retained());
        let events: usize = reader
            .conversations()
            .iter()
            .flat_map(|conversation| &conversation.streams)
            .map(|stream| stream.events.len() * size_of::<crate::tool_stats::Event>())
            .sum();
        assert!(
            days + events <= space,
            "live {days} + {events} past {space} after {unit:?}"
        );
    }
    panic!("finite fixture reader did not settle");
}

/// B3-1b: duplicates charge once: at the least ledger that admits a file
/// once, the same records repeated in one chunk are admitted, and a warm
/// reader that saw them appended equals a fresh scan.
#[test]
fn b3_1b_duplicate_records_charge_once_at_the_ledger_boundary() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    let once = calls(50);
    let whole = events_of_calls(50);
    let path = root.claude("store-a", f::C, once.as_bytes());
    let ledger = smallest_ledger(|ledger| held(&scan_at(&root, ledger), f::C) == whole);
    let mut warm = Reader::new(root.0.clone(), Some(root.home()), caps_at(ledger));
    drain(&mut warm);
    assert_eq!(held(&warm, f::C), whole, "admitted once");
    Fixture::rewrite(&path, once.repeat(16).as_bytes());
    assert_eq!(
        held(&scan_at(&root, ledger), f::C),
        whole,
        "repeats in one chunk charge once"
    );
    drain(&mut warm);
    assert_eq!(held(&warm, f::C), whole);
    equals_fresh_at(&root, &warm, ledger);
}

/// B3-1b: identical retire records add no discovery charge after the first.
#[test]
fn b3_1b_repeated_retire_records_charge_once() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::D, Some("work"), "store-a");
    let summary = format!(
        "tool=claude profile=fixture-profile config_home={} config_home_base=",
        root.store("store-a").display()
    );
    let retire = format!(
        "{{\"ts\":\"2026-11-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"old-worker\",\"target_slot\":\"spawned.0\",\"ref\":\"{}\",\"summary\":\"{summary}\"}}\n",
        f::C
    );
    let journal = root.live("alpha", &meta, &retire).join("events.jsonl");
    root.claude("store-a", f::D, b"");
    root.claude("store-a", f::C, calls(1).as_bytes());
    let complete = |reader: &Reader| !global(&tools(reader), &GapKind::Unreadable);
    // The least ledger at which the single-retire fixture is complete AND
    // holds its one call (two events): both must stay true with 2000.
    let ledger = smallest_ledger(|ledger| {
        let scan = scan_at(&root, ledger);
        complete(&scan) && held(&scan, f::C) == events_of_calls(1)
    });
    Fixture::rewrite(&journal, retire.repeat(2000).as_bytes());
    let fresh = scan_at(&root, ledger);
    assert!(
        complete(&fresh),
        "2000 identical retires are one membership"
    );
    assert_eq!(held(&fresh, f::C), events_of_calls(1));
}

/// B3-7: refusals are not sticky: once an earlier conversation shrinks, a
/// later one it excluded is admitted again, as in a fresh scan.
#[test]
fn b3_7_shrunk_earlier_conversation_readmits_the_later_one() {
    let root = Fixture::new();
    let alpha = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    let beta = root.meta("beta", f::T, "claude", f::D, Some("work"), "store-b");
    root.live("alpha", &alpha, "");
    root.live("beta", &beta, "");
    let path = root.claude("store-a", f::C, calls(400).as_bytes());
    let (large, one) = (events_of_calls(400), events_of_calls(1));
    root.claude("store-b", f::D, calls(1).as_bytes());
    let ledger = smallest_ledger(|ledger| held(&scan_at(&root, ledger), f::C) == large);
    let mut warm = Reader::new(root.0.clone(), Some(root.home()), caps_at(ledger));
    drain(&mut warm);
    assert_eq!(
        (held(&warm, f::C), held(&warm, f::D)),
        (large, 0),
        "the large earlier conversation excludes the later one"
    );
    Fixture::rewrite(&path, calls(1).as_bytes());
    drain(&mut warm);
    assert_eq!((held(&warm, f::C), held(&warm, f::D)), (one, one));
    equals_fresh_at(&root, &warm, ledger);
}

/// B3-7 r3: an unchanged warm round at the least ledger that admits a fresh
/// scan keeps what the fresh scan keeps: discovery pressure on the cached
/// observations is temporary, never a terminal refusal.
#[test]
fn b3_7_unchanged_warm_round_at_the_least_ledger_equals_fresh() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    root.claude("store-a", f::C, calls(400).as_bytes());
    let whole = events_of_calls(400);
    let ledger = smallest_ledger(|ledger| held(&scan_at(&root, ledger), f::C) == whole);
    let mut warm = Reader::new(root.0.clone(), Some(root.home()), caps_at(ledger));
    drain(&mut warm);
    assert_eq!(held(&warm, f::C), whole, "the first round admits it");
    drain(&mut warm);
    assert_eq!(held(&warm, f::C), whole, "the unchanged round keeps it");
    equals_fresh_at(&root, &warm, ledger);
}

/// The bytes the reader owns after a unit, measured without its ledger
/// accounting: every conversation's fold charge (a refused one's kept record
/// too), its files' state, the refusal entries, the task queue's live
/// allocation and the day cache. The ledger must charge at least that.
fn owned_within_charged(reader: &Reader, unit: &Unit) {
    let Some(round) = reader.round.as_ref() else {
        return;
    };
    let conversations: usize = reader
        .conversations()
        .iter()
        .map(|conversation| crate::tool_stats::weigh(conversation, &reader.caps).charge)
        .sum();
    let files: usize = reader
        .files
        .iter()
        .map(|(place, tracked)| super::tracked_bytes(place, tracked))
        .sum();
    let refusals: usize = reader
        .refused
        .keys()
        .map(|(_, id)| {
            size_of::<((crate::tool_stats::Adapter, String), super::Refusal)>() + id.len()
        })
        .sum();
    let queue = round.tasks.capacity() * size_of::<super::Task>();
    let owned = conversations + files + refusals + queue + round.days.retained();
    let charged = round.bytes + reader.held();
    assert!(
        owned <= charged,
        "owned {owned} (conversations {conversations}, files {files}, refusals {refusals}, \
         queue {queue}) past charged {charged} after {unit:?}"
    );
}

/// Steps one round, checking [`owned_within_charged`] after every unit.
fn drain_checked(reader: &mut Reader) {
    reader.round();
    for _ in 0..20_000 {
        let Some(unit) = reader.step() else {
            return;
        };
        owned_within_charged(reader, &unit);
    }
    panic!("finite fixture reader did not settle");
}

/// B3-1a r3: a drained task queue keeps its allocation; the ledger charges
/// it while it is live. 3000 empty project directories queue 3000 project
/// listings at once.
#[test]
fn b3_1a_drained_queue_allocation_stays_charged() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    root.claude("store-a", f::C, calls(1).as_bytes());
    let projects = root.store("store-a").join("projects");
    for n in 0..3000 {
        fs::create_dir_all(projects.join(format!("p{n:04}"))).expect("empty project");
    }
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE);
    drain_checked(&mut reader);
    assert_eq!(held(&reader, f::C), events_of_calls(1));
}

/// B3-1a r3: a conversation refused alone keeps its record (memberships,
/// streams, gaps) and its refusal entry; the ledger charges them.
#[test]
fn b3_1a_refused_records_stay_charged() {
    let root = Fixture::new();
    let store = root.store("store-a");
    let mut meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    for n in 0..20 {
        let id = format!("55555555-5555-4555-8555-{n:012x}");
        meta.push_str(&seat(&format!("w{n}"), "claude", &store, &id));
        root.claude("store-a", &id, calls(150).as_bytes());
    }
    root.live("alpha", &meta, "");
    root.claude("store-a", f::C, calls(1).as_bytes());
    let caps = Caps {
        per_conversation: 100,
        ..Caps::SLICE
    };
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), caps);
    drain_checked(&mut reader);
    assert_eq!(held(&reader, f::C), events_of_calls(1));
    assert_eq!(
        reader.refused.len(),
        20,
        "each 151-event conversation is over 100"
    );
}

/// The membership strings the reader retains, counted outside its ledger
/// accounting: every session, slot and client label held by the round's
/// association and by the conversations.
fn membership_strings(reader: &Reader) -> usize {
    let bytes = |membership: &crate::tool_stats::Membership| {
        membership.session.as_ref().map_or(0, String::len)
            + membership.slot.len()
            + match &membership.client {
                crate::meta::RecordedClient::Label(label) => label.len(),
                crate::meta::RecordedClient::Missing | crate::meta::RecordedClient::Invalid => 0,
            }
    };
    let discovery: usize = reader.round.as_ref().map_or(0, |round| {
        round
            .assoc
            .values()
            .flat_map(|assoc| &assoc.memberships)
            .map(bytes)
            .sum()
    });
    let held: usize = reader
        .conversations()
        .iter()
        .flat_map(|conversation| &conversation.memberships)
        .map(bytes)
        .sum();
    discovery + held
}

/// B3-1a r4: hostile session metas cannot make the retained membership
/// strings exceed the reader's share of the cap. 170 sessions each hold one
/// seat whose slot is 200,000 bytes, all naming the same conversation; the
/// strings alone, kept twice, would pass 64 MiB. Each meta names the slot
/// three times (seat, agent, conversation), about 600 KB, under the 1 MiB
/// meta cap (asserted: a meta over it is truncated and never read, which
/// would make this case pass without testing anything).
///
/// Expected (hand count): each meta's hostile seat is one membership of C
/// over the field limit and is not kept, so the gap line counts 170. Each
/// meta's main seat is kept: C holds 170 memberships, one per meta, each
/// with slot `main` and that meta's distinct session id, and none with the
/// hostile slot. No client is proven, and the gap makes the view Partial
/// (the frozen B2 rule that every gap does).
#[test]
fn b3_1a_hostile_membership_strings_stay_inside_the_cap() {
    let root = Fixture::new();
    let slot = "x".repeat(200_000);
    for n in 0..170 {
        let session = format!("{n:08x}-1111-4111-8111-111111111111");
        let meta = root.meta(&format!("s{n}"), &session, "claude", f::C, None, "store-a")
            + &format!(
                "seat.{slot}=w\nagent_bin.{slot}=claude\nharness_session.{slot}={}\n",
                f::C
            );
        assert!(
            u64::try_from(meta.len()).is_ok_and(|len| len < crate::usage::SESSION_META_MAX_BYTES),
            "precondition: fixture meta {} bytes is read whole",
            meta.len()
        );
        root.live(&format!("s{n}"), &meta, "");
    }
    root.claude("store-a", f::C, calls(1).as_bytes());
    let caps = Caps::SLICE;
    let space = caps.ledger - VIEW_RESERVE;
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), caps);
    reader.round();
    for _ in 0..20_000 {
        let Some(unit) = reader.step() else {
            let view = tools(&reader);
            let lines: Vec<String> = view
                .gaps
                .iter()
                .filter(|line| line.conversation.is_none())
                .map(crate::tool_stats::GapLine::line)
                .collect();
            assert!(
                lines.contains(&"membership over field limit 170".to_owned()),
                "the memberships not kept are a gap naming its reason: {lines:?}"
            );
            assert_eq!(gap(&view, &GapKind::Unreadable), 0, "no IO failed");
            assert!(view.partial(), "the field-limit gap makes the view Partial");
            let kept: Vec<&Conversation> = reader
                .conversations()
                .iter()
                .filter(|conversation| conversation.id == f::C)
                .collect();
            assert_eq!(kept.len(), 1);
            let sessions: BTreeSet<Option<&str>> = kept[0]
                .memberships
                .iter()
                .map(|membership| membership.session.as_deref())
                .collect();
            assert_eq!(kept[0].memberships.len(), 170, "one main seat per meta");
            assert_eq!(sessions.len(), 170, "each meta's own session id");
            assert!(
                kept[0]
                    .memberships
                    .iter()
                    .all(|membership| membership.slot == "main"),
                "no hostile slot is kept"
            );
            assert!(!kept[0].evidence_complete, "no client is proven");
            return;
        };
        let strings = membership_strings(&reader);
        assert!(
            strings <= space,
            "membership strings {strings} past {space} after {unit:?}"
        );
    }
    panic!("finite fixture reader did not settle");
}

/// The bytes the retained memberships own at least, counted outside the
/// reader's accounting from what is allocated: each membership vector's
/// capacity in struct slots and each string's capacity, in the round's
/// association and in the published conversations (B3 review r6: spare
/// slots are retained memory too).
fn membership_floor(reader: &Reader) -> usize {
    fn allocated(memberships: &Vec<crate::tool_stats::Membership>) -> usize {
        memberships.capacity() * size_of::<crate::tool_stats::Membership>()
            + memberships
                .iter()
                .map(|membership| {
                    membership.session.as_ref().map_or(0, String::capacity)
                        + membership.slot.capacity()
                        + match &membership.client {
                            crate::meta::RecordedClient::Label(label) => label.capacity(),
                            crate::meta::RecordedClient::Missing
                            | crate::meta::RecordedClient::Invalid => 0,
                        }
                })
                .sum::<usize>()
    }
    let discovery: usize = reader.round.as_ref().map_or(0, |round| {
        round
            .assoc
            .values()
            .map(|assoc| allocated(&assoc.memberships))
            .sum()
    });
    let published: usize = reader
        .conversations()
        .iter()
        .map(|conversation| allocated(&conversation.memberships))
        .sum();
    discovery + published
}

/// Memberships the round's association holds now.
fn associated(reader: &Reader) -> usize {
    reader.round.as_ref().map_or(0, |round| {
        round
            .assoc
            .values()
            .map(|assoc| assoc.memberships.len())
            .sum()
    })
}

/// B3-1a r5: many memberships whose fields are all within `FIELD_MAX` cannot
/// make the two retained copies (association and the published
/// conversation) exceed a small raw cap. 8 metas, each with a main seat on
/// C and 1000 more seats on C whose slots are exactly `FIELD_MAX` bytes and
/// distinct, each with its own seat name (a repeated name drops those
/// seats); the supplied payload (about 3 MB) passes a 2 MiB cap, so only
/// admission bounds what is kept. A warm round then equals a fresh scan at
/// the same cap.
#[test]
fn b3_1a_many_in_limit_memberships_stay_inside_a_small_cap() {
    use std::fmt::Write as _;
    const METAS: usize = 8;
    const SEATS: usize = 1000;
    let root = Fixture::new();
    let mut supplied = 0;
    for n in 0..METAS {
        let session = format!("{n:08x}-3333-4333-8333-333333333333");
        let mut meta = root.meta(&format!("s{n}"), &session, "claude", f::C, None, "store-a");
        for seat in 0..SEATS {
            let slot = format!("{seat:04}{}", "y".repeat(super::FIELD_MAX - 4));
            assert_eq!(slot.len(), super::FIELD_MAX, "precondition: in limit");
            write!(
                meta,
                "seat.{slot}=w{seat}\nagent_bin.{slot}=claude\nharness_session.{slot}={}\n",
                f::C
            )
            .expect("fixture meta text");
            supplied += size_of::<crate::tool_stats::Membership>() + slot.len() + session.len();
        }
        assert!(
            u64::try_from(meta.len()).is_ok_and(|len| len < crate::usage::SESSION_META_MAX_BYTES),
            "precondition: fixture meta {} bytes is read whole",
            meta.len()
        );
        root.live(&format!("s{n}"), &meta, "");
    }
    root.claude("store-a", f::C, calls(1).as_bytes());
    let space = 2 << 20;
    let caps = caps_at(VIEW_RESERVE + space);
    assert!(
        supplied > space,
        "precondition: the payload ({supplied} B) passes the cap, so admission alone bounds it"
    );
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), caps);
    let mut peak = 0;
    for round in 0..2 {
        reader.round();
        let mut settled = false;
        for _ in 0..20_000 {
            let Some(unit) = reader.step() else {
                settled = true;
                break;
            };
            peak = peak.max(associated(&reader));
            let floor = membership_floor(&reader);
            assert!(
                floor <= space,
                "round {round}: memberships own {floor} B past {space} after {unit:?}"
            );
            every_kept_conversation_has_memberships(&reader);
        }
        assert!(settled, "finite fixture reader did not settle");
    }
    assert!(
        peak >= SEATS / 2,
        "precondition: in-limit memberships are admitted (peak {peak})"
    );
    let fresh = scan(root.0.clone(), Some(root.home()), caps);
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        let asked = Ask {
            page,
            ..ask(Scope::All)
        };
        assert_eq!(
            reader.view(&asked),
            fresh.view(&asked),
            "warm equals fresh {page:?}"
        );
    }
}

/// B3-1a r6: the spare slots of the membership vectors are retained memory
/// too, so admission covers them. 160 metas, each with 129 seats (unique
/// names, short slots) on its own conversation: each conversation gathers
/// 129 memberships from one source, one past a doubling boundary, so its
/// association vector holds about twice the slots it uses. Short strings
/// make the struct slots the larger share. The floor counts capacity
/// (`membership_floor`) after every unit of two rounds, under a 2 MiB raw
/// cap; a warm round then equals a fresh scan at the same cap.
#[test]
fn b3_1a_membership_vector_slack_stays_inside_a_small_cap() {
    use std::fmt::Write as _;
    const METAS: usize = 160;
    const SEATS: usize = 129;
    let root = Fixture::new();
    let mut supplied = 0;
    for n in 0..METAS {
        let session = format!("{n:08x}-5555-4555-8555-555555555555");
        let conversation = format!("{n:08x}-6666-4666-8666-666666666666");
        let mut meta = format!("meta_version=3\nsession=s{n}\nsession_id={session}\n");
        for seat in 0..SEATS {
            let slot = format!("k{seat}");
            assert!(slot.len() <= super::FIELD_MAX, "precondition: in limit");
            write!(
                meta,
                "seat.{slot}=w{seat}\nagent_bin.{slot}=claude\nharness_session.{slot}={conversation}\n"
            )
            .expect("fixture meta text");
            supplied += size_of::<crate::tool_stats::Membership>() + slot.len() + session.len();
        }
        assert!(
            u64::try_from(meta.len()).is_ok_and(|len| len < crate::usage::SESSION_META_MAX_BYTES),
            "precondition: fixture meta {} bytes is read whole",
            meta.len()
        );
        root.live(&format!("s{n}"), &meta, "");
    }
    let space = 2 << 20;
    let caps = caps_at(VIEW_RESERVE + space);
    assert!(
        supplied > space,
        "precondition: the payload ({supplied} B) passes the cap, so admission alone bounds it"
    );
    let mut reader = Reader::new(root.0.clone(), Some(root.home()), caps);
    let mut peak = 0;
    for round in 0..2 {
        reader.round();
        let mut settled = false;
        for _ in 0..20_000 {
            let Some(unit) = reader.step() else {
                settled = true;
                break;
            };
            peak = peak.max(associated(&reader));
            let floor = membership_floor(&reader);
            assert!(
                floor <= space,
                "round {round}: memberships own {floor} B past {space} after {unit:?}"
            );
            every_kept_conversation_has_memberships(&reader);
        }
        assert!(settled, "finite fixture reader did not settle");
    }
    assert!(
        peak >= 4 * SEATS,
        "precondition: several whole conversations are admitted (peak {peak})"
    );
    let fresh = scan(root.0.clone(), Some(root.home()), caps);
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        let asked = Ask {
            page,
            ..ask(Scope::All)
        };
        assert_eq!(
            reader.view(&asked),
            fresh.view(&asked),
            "warm equals fresh {page:?}"
        );
    }
}

/// Lead ruling ae-20261108T170544Z-0d20805a (3): every conversation the
/// reader keeps has its memberships, so no View (`Reader::view` folds
/// exactly `Reader::conversations`) sees one without them; a conversation
/// association displaces is removed whole.
fn every_kept_conversation_has_memberships(reader: &Reader) {
    for conversation in reader.conversations() {
        assert!(
            !conversation.memberships.is_empty(),
            "conversation {} kept without its memberships",
            conversation.id
        );
    }
}

/// Lead ruling ae-20261108T170544Z-0d20805a (1), (2): refused records do
/// not narrow the next round's discovery. Six conversations, each the main
/// seat of its own session, each with a 300-call transcript. At the least
/// ledger at which a fresh scan's discovery is complete (no global gap),
/// the transcripts do not fit, so round 1 ends with refused records pinned.
/// A warm round 2 at that exact ledger equals the fresh scan.
#[test]
fn b3_7_refused_records_give_way_at_the_least_complete_discovery_ledger() {
    let root = Fixture::new();
    for n in 0..6 {
        let session = format!("{n:08x}-7777-4777-8777-777777777777");
        let conversation = format!("{n:08x}-8888-4888-8888-888888888888");
        let meta = root.meta(
            &format!("s{n}"),
            &session,
            "claude",
            &conversation,
            None,
            "store-a",
        );
        root.live(&format!("s{n}"), &meta, "");
        root.claude("store-a", &conversation, calls(300).as_bytes());
    }
    let complete = |reader: &Reader| !global(&tools(reader), &GapKind::Unreadable);
    let ledger = smallest_ledger(|ledger| complete(&scan_at(&root, ledger)));
    let fresh = scan_at(&root, ledger);
    assert!(
        gap(&tools(&fresh), &GapKind::Ledger) > 0,
        "precondition: the transcripts do not all fit at this ledger"
    );
    let mut warm = Reader::new(root.0.clone(), Some(root.home()), caps_at(ledger));
    drain(&mut warm);
    assert!(
        !warm.refused.is_empty(),
        "precondition: round 1 ends with refused records pinned"
    );
    warm.round();
    for _ in 0..20_000 {
        if warm.step().is_none() {
            break;
        }
        every_kept_conversation_has_memberships(&warm);
    }
    assert!(!warm.busy(), "finite fixture reader did not settle");
    assert!(complete(&warm), "round 2 discovery is complete, as fresh");
    equals_fresh_at(&root, &warm, ledger);
}
