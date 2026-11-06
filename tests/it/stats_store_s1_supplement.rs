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
