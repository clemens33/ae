//! Driver supplements to the frozen S1 store spec: a valid document under
//! another version is refused, and the fuzz harness's heavy journal mode runs
//! in the gate, not only under cargo-fuzz.

use ae::stats_store::{Store, parse_view, render_view};

const S: &str = "11111111-1111-4111-8111-111111111111";

/// A store holding one certificate and its link, read whole.
fn store() -> Store {
    let journal = format!(
        "{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_session_id\":\"{S}\",\"birth_v\":1,\"birth\":\"aaaaaaaa-aaaa-4aaa-8aaa-000000000001\",\"birth_tool\":\"codex\"}}\n\
         {{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"spawn\",\"target\":\"scout\",\"birth\":\"aaaaaaaa-aaaa-4aaa-8aaa-000000000001\"}}\n"
    );
    let mut store = Store::default();
    let meta = format!("session_id={S}\n");
    assert!(store.live_source("a", "live", meta.as_bytes()).is_ok());
    assert!(store.chunk("a", 0, journal.as_bytes(), true).is_ok());
    store
}

#[test]
fn a_valid_render_under_another_version_is_refused() {
    let store = store();
    let Ok(evidence) = store.render() else {
        panic!("a bounded store renders");
    };
    assert!(
        Store::parse(&evidence).is_ok(),
        "the unaltered render parses"
    );
    let view = render_view(&store.view(Some(S)));
    assert!(parse_view(&view).is_ok(), "the unaltered view parses");
    for version in ["0", "2", "-1", "\"1\""] {
        let other = evidence.replacen(
            "{\"stats_store\":1,",
            &format!("{{\"stats_store\":{version},"),
            1,
        );
        assert_ne!(other, evidence, "the version member was rewritten");
        assert!(Store::parse(&other).is_err(), "evidence version {version}");
        let other = view.replacen(
            "{\"stats_view\":1}",
            &format!("{{\"stats_view\":{version}}}"),
            1,
        );
        assert_ne!(other, view, "the version member was rewritten");
        assert!(parse_view(&other).is_err(), "view version {version}");
    }
}

#[test]
fn the_fuzz_harness_journal_mode_runs_on_mode_zero_inputs() {
    let tracked = include_bytes!("../../fuzz/seeds/stats_store/mode0-journal");
    assert_eq!(
        tracked[0] % 16,
        0,
        "the tracked seed selects the journal mode"
    );
    let mut generated = Vec::new();
    for first in [0_u8, 16, 0x40, 0xf0] {
        let mut data = vec![first];
        data.extend_from_slice(&tracked[1..]);
        generated.push(data);
    }
    generated.push(vec![0]);
    generated.push(vec![0x20, 0, 0, b'\n', 0]);
    crate::stats_store_s1_spec::fuzz_case(tracked);
    for data in &generated {
        crate::stats_store_s1_spec::fuzz_case(data);
    }
}

#[test]
fn a_date_the_calendar_does_not_have_is_refused_and_year_zero_is_kept() {
    let memo = "{\"ts\":\"2026-10-02T10:00:00Z\",\"actor\":\"lead\",\"action\":\"memo\"}\n";
    let mut store = Store::default();
    let meta = format!("session_id={S}\n");
    assert!(store.live_source("a", "live", meta.as_bytes()).is_ok());
    assert!(store.chunk("a", 0, memo.as_bytes(), true).is_ok());
    let Ok(evidence) = store.render() else {
        panic!("a bounded store renders");
    };
    let view = render_view(&store.view(Some(S)));
    // Evidence holds the range once; the view holds it in both scopes.
    assert_eq!(evidence.matches("\"2026-10-02\"").count(), 2, "{evidence}");
    assert_eq!(view.matches("\"2026-10-02\"").count(), 4, "{view}");
    let swap = |text: &str, date: &str| text.replace("\"2026-10-02\"", &format!("\"{date}\""));
    for date in [
        "2026-02-30",
        "2026-13-01",
        "2026-00-00",
        "2025-02-29",
        "1900-02-29",
    ] {
        assert!(
            Store::parse(&swap(&evidence, date)).is_err(),
            "evidence {date}"
        );
        assert!(parse_view(&swap(&view, date)).is_err(), "view {date}");
    }
    for date in ["0000-02-29", "0000-01-01", "2024-02-29", "9999-12-31"] {
        assert!(
            Store::parse(&swap(&evidence, date)).is_ok(),
            "evidence {date}"
        );
        assert!(parse_view(&swap(&view, date)).is_ok(), "view {date}");
    }
}

/// One live source of session `S` holding `journal`, read whole by the
/// unchanged fold: the oracle for a store that has consumed all of it.
fn folded(journal: &str) -> ae::stats::View {
    let meta = format!("session_id={S}\n");
    ae::stats::fold(
        vec![ae::stats::Source::live(
            "live",
            meta.as_bytes(),
            Ok(journal.as_bytes().to_vec()),
        )],
        Some(S),
    )
}

fn memo(date: &str) -> String {
    format!("{{\"ts\":\"{date}T10:00:00Z\",\"actor\":\"lead\",\"action\":\"memo\"}}\n")
}

#[test]
fn the_range_of_records_that_keep_only_a_date_is_their_earliest_to_latest() {
    // Out of order on purpose: the first dated line is neither end.
    let journal = [memo("2026-10-03"), memo("2026-10-05"), memo("2026-10-01")].concat();
    let mut store = Store::default();
    let meta = format!("session_id={S}\n");
    assert!(store.live_source("a", "live", meta.as_bytes()).is_ok());
    assert!(store.chunk("a", 0, journal.as_bytes(), true).is_ok());
    let expected = folded(&journal);
    assert_eq!(
        expected.all.range,
        Some(("2026-10-01".to_owned(), "2026-10-05".to_owned()))
    );
    assert_eq!(store.view(Some(S)), expected);
    let Ok(text) = store.render() else {
        panic!("a bounded store renders");
    };
    let Ok(reloaded) = Store::parse(&text) else {
        panic!("its own render parses");
    };
    assert_eq!(reloaded.view(Some(S)), expected);
}

#[test]
fn an_event_at_no_line_start_is_dropped_once_the_ordered_read_passes_it() {
    let journal = format!("{}\n{}", certificate(1), memo("2026-10-02"));
    let mut store = Store::default();
    let meta = format!("session_id={S}\n");
    assert!(store.live_source("a", "live", meta.as_bytes()).is_ok());
    // Offset 5 is inside the first line: no record of the journal starts there.
    assert!(store.event("a", 5, certificate(2).as_bytes()).is_ok());
    assert_eq!(
        store.view(Some(S)).all.total,
        1,
        "the event counts ahead of the cursor"
    );
    assert!(store.chunk("a", 0, journal.as_bytes(), true).is_ok());
    // Caught up, the store answers what the journal holds and nothing else.
    assert_eq!(store.view(Some(S)), folded(&journal));
    assert_eq!(store.view(Some(S)).all.total, 1);
}

/// A certificate of birth `n` in session `S`, LF excluded.
fn certificate(n: u32) -> String {
    format!(
        "{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_session_id\":\"{S}\",\"birth_v\":1,\"birth\":\"aaaaaaaa-aaaa-4aaa-8aaa-{n:012x}\",\"birth_tool\":\"codex\"}}"
    )
}

/// A store with one live source of `S` and nothing read yet.
fn fresh(limit: usize) -> Store {
    let mut store = Store::with_line_limit(limit);
    let meta = format!("session_id={S}\n");
    assert!(store.live_source("a", "live", meta.as_bytes()).is_ok());
    store
}

fn rendered(store: &Store) -> String {
    store
        .render()
        .unwrap_or_else(|_| panic!("a bounded store renders"))
}

#[test]
fn an_event_is_one_line_within_the_inclusive_line_limit() {
    let line = certificate(1);
    let exact = fresh(line.len());
    let mut at_limit = exact.clone();
    assert!(
        at_limit.event("a", 0, line.as_bytes()).is_ok(),
        "a line of exactly the limit is read"
    );
    assert_eq!(at_limit.view(Some(S)).all.total, 1);
    let mut over = fresh(line.len() - 1);
    let before = rendered(&over);
    assert!(
        over.event("a", 0, line.as_bytes()).is_err(),
        "an event longer than the limit is refused and records nothing, not even a line-limit gap"
    );
    assert_eq!(rendered(&over), before);
    let mut store = fresh(1024);
    let before = rendered(&store);
    for split in [format!("{line}\n"), line.replacen(',', ",\n", 1)] {
        assert!(
            store.event("a", 0, split.as_bytes()).is_err(),
            "not one line: {split:?}"
        );
        assert_eq!(rendered(&store), before);
    }
}

#[test]
fn an_event_behind_the_cursor_changes_nothing_even_when_it_differs_from_the_read() {
    let journal = memo("2026-10-02");
    let mut store = fresh(1024);
    assert!(store.chunk("a", 0, journal.as_bytes(), true).is_ok());
    let before = rendered(&store);
    // The ordered read kept only a date at offset 0; a certificate there is late news.
    assert!(store.event("a", 0, certificate(1).as_bytes()).is_ok());
    assert_eq!(rendered(&store), before);
    assert_eq!(store.view(Some(S)), folded(&journal));
}

#[test]
fn an_event_for_a_source_whose_meta_is_not_text_keeps_nothing() {
    let mut store = Store::default();
    assert!(store.live_source("a", "live", &[0xff, b'\n']).is_ok());
    let before = rendered(&store);
    assert!(store.event("a", 0, certificate(1).as_bytes()).is_ok());
    assert_eq!(rendered(&store), before);
    assert_eq!(store.view(None).all.total, 0);
    assert!(Store::parse(&rendered(&store)).is_ok());
}

#[test]
fn an_empty_line_inside_a_chunk_does_not_end_it() {
    let journal = format!(
        "{}\n\n{}\n\n{}",
        certificate(1),
        certificate(2),
        memo("2026-10-05").trim_end()
    );
    let mut store = fresh(1024);
    let end = store.chunk("a", 0, journal.as_bytes(), true);
    assert_eq!(end, Ok(u64::try_from(journal.len()).unwrap_or(u64::MAX)));
    assert_eq!(store.view(Some(S)), folded(&journal));
    assert_eq!(store.view(Some(S)).all.total, 2);
}
