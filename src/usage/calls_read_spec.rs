//! #288 A3-A8 through real association, filesystem discovery and bounded reads.
//! Native logs are synthetic. Fresh-scan comparisons supplement hand counts.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned acceptance fixture IO and assertions"
)]

use super::{LSTATS, Reader, STEP, Unit, UnitKind, scan};
use crate::stats::Cell;
use crate::tool_stats::{Ask, Caps, Cols, Filter, GapKind, Page, Scope, State, View};
use crate::tool_stats_fixture::{self as f, Fixture};
use std::fs;

fn ask(page: Page) -> Ask {
    Ask {
        page,
        scope: Scope::All,
        filter: Filter::All,
    }
}

fn new(root: &Fixture) -> Reader {
    Reader::new(root.0.clone(), Some(root.home()), Caps::SLICE)
}

fn drain(reader: &mut Reader) -> Vec<Unit> {
    let mut units = Vec::new();
    for _ in 0..20_000 {
        let Some(unit) = reader.step() else {
            assert!(!reader.busy());
            return units;
        };
        assert!(unit.bytes <= STEP, "actual door bytes: {unit:?}");
        assert!(unit.lstats <= LSTATS, "actual stat calls: {unit:?}");
        assert!(unit.listings <= 1, "one listing per unit: {unit:?}");
        units.push(unit);
    }
    panic!("finite fixture reader did not settle");
}

fn round(reader: &mut Reader) -> Vec<Unit> {
    reader.round();
    drain(reader)
}

fn attempts(view: &View) -> u64 {
    match view.totals {
        Cols::Tool(cols) => cols.attempts,
        other => panic!("Tools: {other:?}"),
    }
}

fn returned(view: &View) -> u64 {
    match view.totals {
        Cols::Tool(cols) => cols.returned,
        other => panic!("Tools: {other:?}"),
    }
}

fn gap(view: &View, kind: &GapKind) -> bool {
    view.gaps
        .iter()
        .any(|line| line.kind == *kind && line.count > 0)
}

fn equal_fresh(root: &Fixture, cached: &Reader, count: u64) {
    let fresh = scan(root.0.clone(), Some(root.home()), Caps::SLICE);
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        assert_eq!(
            cached.view(&ask(page)),
            fresh.view(&ask(page)),
            "cached equals fresh {page:?}"
        );
    }
    assert_eq!(
        attempts(&cached.view(&ask(Page::Tools))),
        count,
        "independent native call hand count"
    );
}

fn seeded() -> (Fixture, Reader, std::path::PathBuf) {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    root.live("alpha", &meta, "");
    let text = f::call("first", "first", "Read", "model-a", "{}");
    let path = root.claude("store-a", f::C, text.as_bytes());
    let mut reader = new(&root);
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    (root, reader, path)
}

#[test]
fn a4_append_partial_line_rewrite_delete_replace_and_truncate_regrow_match_hand_counts() {
    let (root, mut reader, path) = seeded();
    let mut bytes = fs::read_to_string(&path).expect("native fixture");
    bytes.push_str(&f::result("first", false, None));
    Fixture::rewrite(&path, bytes.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    assert_eq!(returned(&reader.view(&ask(Page::Tools))), 1);

    let second = f::call("second", "second", "Read", "model-a", "{}");
    let split = second.len() / 2;
    let prefix = bytes.clone() + &second[..split];
    Fixture::rewrite(&path, prefix.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    bytes.push_str(&second);
    Fixture::rewrite(&path, bytes.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 2);

    let changed = bytes.replacen("Read", "Bash", 1);
    assert_eq!(changed.len(), bytes.len());
    Fixture::rewrite(&path, changed.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 2);
    assert!(format!("{:?}", reader.view(&ask(Page::Tools)).rows).contains("Bash"));

    fs::remove_file(&path).expect("delete fixture native log");
    round(&mut reader);
    assert_eq!(attempts(&reader.view(&ask(Page::Tools))), 2);
    assert!(gap(&reader.view(&ask(Page::Tools)), &GapKind::SourceGone));
    assert!(reader.view(&ask(Page::Tools)).partial());
    drop(reader);
    // S5 owns the payload. Establish its epoch through a real publication.
    Fixture::rewrite(
        &root.0.join("sessions/alpha/events.jsonl"),
        f::event("ordinary", "epoch fixture").as_bytes(),
    );
    let mut maintainer = crate::stats_store::maint::Maintainer::new(
        &root.0,
        crate::stats_store::disk::LIMITS,
        std::time::Duration::from_millis(100),
    );
    for _ in 0..100 {
        let _ = maintainer.step();
        if root.0.join("stats/epoch").is_file() {
            break;
        }
    }
    assert!(
        root.0.join("stats/epoch").is_file(),
        "real S5 epoch before reset"
    );
    drop(maintainer);
    assert!(!matches!(
        crate::stats_store::disk::reset(&root.0, std::time::Duration::ZERO)
            .expect("P6 reset rebuilds existing sources"),
        crate::stats_store::disk::Reset::Busy
    ));
    let mut reader = new(&root);
    round(&mut reader);
    equal_fresh(&root, &reader, 0);
    Fixture::rewrite(&path, changed.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 2);

    let replacement = path.with_extension("replacement");
    Fixture::rewrite(&replacement, changed.as_bytes());
    fs::rename(&replacement, &path).expect("replace fixture inode");
    round(&mut reader);
    equal_fresh(&root, &reader, 2);
    Fixture::rewrite(&path, second.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    let regrown = second.clone() + &f::call("third", "third", "Bash", "model-b", "{}");
    Fixture::rewrite(&path, regrown.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 2);
}

#[test]
fn a4_same_length_edit_far_before_cursor_is_not_hidden_by_an_unchanged_tail() {
    let (root, mut reader, path) = seeded();
    let first = fs::read_to_string(&path).expect("fixture call");
    let padding = format!(
        "{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}\n",
        "x".repeat(16_384)
    );
    let original = first + &padding;
    Fixture::rewrite(&path, original.as_bytes());
    round(&mut reader);
    let changed = original.replacen("Read", "Bash", 1);
    assert_eq!(original.len(), changed.len());
    assert_eq!(
        &original[original.len() - 4096..],
        &changed[changed.len() - 4096..]
    );
    Fixture::rewrite(&path, changed.as_bytes());
    let units = round(&mut reader);
    assert!(
        units
            .iter()
            .any(|unit| unit.kind == UnitKind::Verify && unit.bytes > 0)
    );
    equal_fresh(&root, &reader, 1);
    let actual = reader.view(&ask(Page::Tools));
    assert!(format!("{:?}", actual.rows).contains("Bash"));
    assert!(!format!("{:?}", actual.rows).contains("Read"));
}

#[test]
fn a4_long_overcap_line_is_one_gap_and_does_not_hide_the_following_call() {
    let (root, mut reader, path) = seeded();
    let mut bytes = vec![b'x'; (4 << 20) * 2 + 17];
    bytes.push(b'\n');
    bytes.extend(f::call("after-long", "after-long", "Read", "model-a", "{}").bytes());
    Fixture::rewrite(&path, &bytes);
    let units = round(&mut reader);
    assert!(
        units
            .iter()
            .filter(|unit| unit.kind == UnitKind::Read)
            .count()
            >= 3
    );
    equal_fresh(&root, &reader, 1);
    let actual = reader.view(&ask(Page::Tools));
    let over: u64 = actual
        .gaps
        .iter()
        .filter(|line| line.kind == GapKind::OverCap)
        .map(|line| line.count)
        .sum();
    assert_eq!(over, 1);
}

#[test]
fn a3_carried_copies_two_reseats_and_old_route_tail_have_one_unknown_client() {
    let root = Fixture::new();
    let alpha = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    let beta = root.meta("beta", f::T, "claude", f::C, Some("personal"), "store-b");
    let journal = f::event(
        "reseat",
        &format!(
            "reseated [from p@old to p@work, conversation {}, carried]",
            f::C
        ),
    ) + &f::event(
        "reseat",
        &format!(
            "reseated [from p@work to p@personal, conversation {}, carried]",
            f::C
        ),
    );
    root.live("alpha", &alpha, &journal);
    root.live("beta", &beta, "");
    let text = f::call("old-route-tail", "old-route-tail", "Read", "model-a", "{}");
    root.claude("store-a", f::C, text.as_bytes());
    root.claude("store-b", f::C, text.as_bytes());
    let mut reader = new(&root);
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    let actual = reader.view(&ask(Page::Tools));
    assert_eq!(actual.detail.len(), 1);
    assert_eq!(actual.detail[0].client, Cell::Unknown);
    for session in [f::S, f::T] {
        let selected = Ask {
            scope: Scope::This(Some(session.into())),
            ..ask(Page::Tools)
        };
        assert_eq!(attempts(&reader.view(&selected)), 1);
    }
}

#[test]
fn a3_discovery_checks_journal_to_eof_beyond_four_mib() {
    let (root, mut reader, _) = seeded();
    let line = f::event("memo", "ordinary padding record");
    let mut journal = line.repeat((4 << 20) / line.len() + 1);
    journal.push_str(&f::event(
        "reseat",
        &format!(
            "reseated [from p@old to p@work, conversation {}, carried]",
            f::C
        ),
    ));
    Fixture::rewrite(
        &root.0.join("sessions/alpha/events.jsonl"),
        journal.as_bytes(),
    );
    let units = round(&mut reader);
    assert!(
        units
            .iter()
            .filter(|unit| unit.kind == UnitKind::Journal)
            .map(|unit| unit.bytes)
            .sum::<u64>()
            > 4 << 20
    );
    equal_fresh(&root, &reader, 1);
    assert!(
        reader
            .view(&ask(Page::Tools))
            .detail
            .iter()
            .all(|row| row.client == Cell::Unknown)
    );
}

#[test]
fn a3_incomplete_or_unreadable_journal_and_disagreeing_memberships_are_unknown() {
    let (root, mut reader, _) = seeded();
    let journal = root.0.join("sessions/alpha/events.jsonl");
    Fixture::rewrite(&journal, b"{\"ts\":\"2026-11-01T00:00:00Z\"");
    round(&mut reader);
    assert_eq!(attempts(&reader.view(&ask(Page::Tools))), 1);
    assert!(reader.view(&ask(Page::Tools)).partial());
    assert!(!reader.view(&ask(Page::Tools)).gaps.is_empty());
    assert!(
        reader
            .view(&ask(Page::Tools))
            .detail
            .iter()
            .all(|row| row.client == Cell::Unknown)
    );
    fs::remove_file(&journal).expect("remove incomplete fixture journal");
    fs::create_dir(&journal).expect("unreadable journal node");
    round(&mut reader);
    assert!(reader.view(&ask(Page::Tools)).partial());
    assert!(!reader.view(&ask(Page::Tools)).gaps.is_empty());
    assert!(!reader.view(&ask(Page::Tools)).detail.is_empty());
    assert!(
        reader
            .view(&ask(Page::Tools))
            .detail
            .iter()
            .all(|row| row.client == Cell::Unknown)
    );
    fs::remove_dir(&journal).expect("restore fixture journal");
    Fixture::rewrite(&journal, b"");
    let beta = root.meta("beta", f::T, "claude", f::C, Some("personal"), "store-a");
    root.live("beta", &beta, "");
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    assert!(
        reader
            .view(&ask(Page::Tools))
            .detail
            .iter()
            .all(|row| row.client == Cell::Unknown)
    );
}

#[test]
fn d10_first_resume_label_relabels_all_events_but_global_definitions_do_not() {
    let (root, mut reader, _) = seeded();
    let dir = root.0.join("sessions/alpha");
    let missing = root.meta("alpha", f::S, "claude", f::C, None, "store-a");
    Fixture::rewrite(&dir.join("meta"), missing.as_bytes());
    round(&mut reader);
    assert_eq!(
        reader.view(&ask(Page::Tools)).detail[0].client,
        Cell::Unknown
    );
    let labeled = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    Fixture::rewrite(&dir.join("meta"), labeled.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    let before = reader.view(&ask(Page::Tools));
    assert_eq!(before.detail[0].client, Cell::Value("work".into()));
    // The recorded store and model stay fixed while today's definitions move.
    let config = root.0.join("config");
    Fixture::rewrite(&config, b"[clients]\nwork = codex config_home=/fixture/different\n[profiles]\nfixture-profile = \"codex --model model-b\"\n");
    let native_config = root.home().join(".ae");
    fs::create_dir(&native_config).expect("fixture global configuration");
    Fixture::rewrite(
        &native_config.join("config"),
        &fs::read(&config).expect("config bytes"),
    );
    round(&mut reader);
    assert_eq!(reader.view(&ask(Page::Tools)), before);
    assert_eq!(before.detail[0].model, Cell::Value("model-a".into()));
    let edited = labeled.replace("client.main=work", "client.main=personal");
    Fixture::rewrite(&dir.join("meta"), edited.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    assert_eq!(
        reader.view(&ask(Page::Tools)).detail[0].client,
        Cell::Value("personal".into())
    );
}

#[test]
fn a3_live_and_archive_memberships_share_counts_and_each_session_scope() {
    let (root, mut reader, _) = seeded();
    let archived = root.meta("old", f::T, "claude", f::C, Some("work"), "store-a");
    root.archive(f::T, &archived);
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    for session in [f::S, f::T] {
        let selected = Ask {
            scope: Scope::This(Some(session.into())),
            ..ask(Page::Tools)
        };
        assert_eq!(attempts(&reader.view(&selected)), 1);
    }
    assert!(
        reader
            .view(&ask(Page::Tools))
            .detail
            .iter()
            .all(|row| row.client == Cell::Unknown)
    );
}

#[test]
fn a6_default_excludes_unassociated_logs_and_discovers_new_native_children() {
    let (root, mut reader, _) = seeded();
    root.claude("store-a", f::D, f::CLASSES.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    let replay = f::call("first", "first", "Read", "model-a", "{}");
    root.child("store-a", f::C, "replay", replay.as_bytes());
    let child = f::call("own-child", "own-child", "Read", "model-child", "{}");
    root.child("store-a", f::C, "independent", child.as_bytes());
    round(&mut reader);
    equal_fresh(&root, &reader, 2);
    let actual = reader.view(&ask(Page::Tools));
    assert!(
        actual.detail.iter().any(
            |row| row.model == Cell::Value("model-child".into()) && row.effort == Cell::Unknown
        )
    );
}

#[test]
fn a5_a6_unsupported_and_missing_native_history_are_coverage_gaps() {
    let root = Fixture::new();
    let unsupported = root.meta("alpha", f::S, "grok", f::C, Some("work"), "store-a");
    root.live("alpha", &unsupported, "");
    let mut reader = new(&root);
    round(&mut reader);
    let actual = reader.view(&ask(Page::Tools));
    assert_eq!(actual.state, State::NoSupported);
    assert!(
        actual
            .gaps
            .iter()
            .any(|line| matches!(&line.kind, GapKind::Unsupported(tool) if tool == "grok"))
    );
    let supported = root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a");
    Fixture::rewrite(&root.0.join("sessions/alpha/meta"), supported.as_bytes());
    round(&mut reader);
    let actual = reader.view(&ask(Page::Tools));
    assert!(actual.partial());
    assert!(gap(&actual, &GapKind::Unlocated));
}

#[test]
fn a8_warm_unchanged_round_does_no_transcript_read_and_all_surfaces_drop_private_paths() {
    let (root, mut reader, _) = seeded();
    let units = round(&mut reader);
    assert!(
        units
            .iter()
            .all(|unit| !matches!(unit.kind, UnitKind::Read | UnitKind::Verify))
    );
    let observations = format!("{:?}", reader.conversations());
    assert!(
        !observations.contains(f::PRIVATE),
        "private located path escaped per-file state"
    );
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        let actual = reader.view(&ask(page));
        assert!(!format!("{actual:?}").contains(f::PRIVATE));
        for line in actual.gaps {
            assert!(!line.line().contains(f::PRIVATE));
        }
    }
    equal_fresh(&root, &reader, 1);
}

#[test]
fn a6_codex_reads_the_middle_chronologically_and_excludes_unassociated_child_history() {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "codex", f::C, Some("work"), "store-a"),
        "",
    );
    let padding = "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"content\":\"ordinary filler\"}}\n";
    let pad = padding.repeat((300 << 10) / padding.len() + 1);
    let bytes = pad.clone()
        + &f::context("middle-turn", "middle-model", "high")
        + &f::codex_call("middle-call", Some("middle-turn"), "read")
        + &f::codex_result("middle-call")
        + &pad;
    root.codex("store-a", f::C, bytes.as_bytes());
    root.codex("store-a", f::D, f::CODEX.as_bytes());
    let mut reader = new(&root);
    let units = round(&mut reader);
    equal_fresh(&root, &reader, 1);
    assert!(
        units
            .iter()
            .filter(|unit| unit.kind == UnitKind::Read)
            .map(|unit| unit.bytes)
            .sum::<u64>()
            >= u64::try_from(bytes.len()).expect("fixture byte count")
    );
    let actual = reader.view(&ask(Page::Tools));
    assert_eq!(returned(&actual), 1);
    assert_eq!(actual.detail[0].model, Cell::Value("middle-model".into()));
    assert_eq!(actual.detail[0].effort, Cell::Value("high".into()));
    assert_eq!(actual.detail[0].client, Cell::Value("work".into()));
}

#[test]
fn a3_prior_and_retire_records_keep_old_conversations_in_scope_with_unknown_client() {
    let root = Fixture::new();
    let meta = root.meta("alpha", f::S, "claude", f::D, Some("work"), "store-a")
        + &format!("harness_session_prior.main=claude:{}\n", f::C);
    root.live("alpha", &meta, "");
    root.claude("store-a", f::D, b"");
    root.claude(
        "store-a",
        f::C,
        f::call("prior", "prior", "Read", "model-a", "{}").as_bytes(),
    );
    let mut reader = new(&root);
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    let actual = reader.view(&Ask {
        scope: Scope::This(Some(f::S.into())),
        ..ask(Page::Tools)
    });
    assert_eq!(attempts(&actual), 1);
    assert_eq!(actual.detail[0].client, Cell::Unknown);
    assert!(reader.conversations().iter().any(|conversation| {
        conversation.id == f::C
            && conversation
                .memberships
                .iter()
                .any(|membership| membership.role == crate::tool_stats::Role::Prior)
    }));

    let without_prior = root.meta("alpha", f::S, "claude", f::D, Some("work"), "store-a");
    Fixture::rewrite(
        &root.0.join("sessions/alpha/meta"),
        without_prior.as_bytes(),
    );
    let summary = format!(
        "tool=claude profile=fixture-profile config_home={} config_home_base=",
        root.store("store-a").display()
    );
    let retire = format!(
        "{{\"ts\":\"2026-11-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"old-worker\",\"target_slot\":\"spawned.0\",\"ref\":\"{}\",\"summary\":\"{summary}\"}}\n",
        f::C
    );
    Fixture::rewrite(
        &root.0.join("sessions/alpha/events.jsonl"),
        retire.as_bytes(),
    );
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    let actual = reader.view(&ask(Page::Tools));
    assert_eq!(actual.detail[0].client, Cell::Unknown);
    assert!(reader.conversations().iter().any(|conversation| {
        conversation.id == f::C
            && conversation
                .memberships
                .iter()
                .any(|membership| membership.role == crate::tool_stats::Role::Retired)
    }));
}

#[test]
fn a4_disagreeing_carried_copies_are_one_call_with_conflicted_model() {
    let (root, mut reader, _) = seeded();
    root.live(
        "beta",
        &root.meta("beta", f::T, "claude", f::C, Some("work"), "store-b"),
        "",
    );
    root.claude(
        "store-b",
        f::C,
        f::call("first", "first", "Read", "model-b", "{}").as_bytes(),
    );
    round(&mut reader);
    equal_fresh(&root, &reader, 1);
    let actual = reader.view(&ask(Page::Tools));
    assert!(gap(&actual, &GapKind::Conflict));
    assert_eq!(actual.detail.len(), 1);
    assert!(actual.detail.iter().all(|row| row.model == Cell::Unknown));
}

#[test]
fn a8_unit_receipts_are_actual_and_match_the_values_returned_by_step() {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        "",
    );
    let bytes = f::call("door", "door", "Read", "model-a", "{}");
    root.claude("store-a", f::C, bytes.as_bytes());
    let _ = super::take_units();
    let mut reader = new(&root);
    let units = round(&mut reader);
    assert_eq!(
        super::take_units(),
        units,
        "receipts come from the work actually completed"
    );
    assert!(units.iter().any(|unit| unit.kind == UnitKind::Read
        && unit.bytes == u64::try_from(bytes.len()).expect("fixture size")
        && unit.opens > 0));
    assert!(units.iter().any(|unit| unit.listings > 0));
    assert!(units.iter().any(|unit| unit.lstats > 0));
    assert!(
        super::take_units().is_empty(),
        "receipt reads drain without IO"
    );
}
