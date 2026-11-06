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
