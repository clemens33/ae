//! Who may clear a vendor block (quarantine2, lead ruling 2): the verdict on
//! one reading of the caller, with every doubt a refusal.

use super::{ClearAsker, Marker, PaneRead, clear_asker, clear_prompt, clear_refusal};

fn human(session: &str) -> PaneRead {
    PaneRead {
        session: Some(session.to_owned()),
        human: true,
    }
}

#[test]
fn only_no_markers_at_all_is_the_terminal() {
    let never = |_: &str| -> Option<PaneRead> { panic!("no pane is read") };
    assert_eq!(
        clear_asker(Marker::Absent, None, never),
        ClearAsker::Terminal
    );
    assert_eq!(
        clear_asker(Marker::Absent, Some(""), never),
        ClearAsker::Terminal
    );
}

#[test]
fn a_partial_or_malformed_marker_is_refused_without_reading_a_pane() {
    let never = |_: &str| -> Option<PaneRead> { panic!("no pane is read") };
    for (marker, pane) in [
        (Marker::Resolved, None),
        (Marker::Absent, Some("%3")),
        (Marker::Malformed, None),
        (Marker::Malformed, Some("%3")),
    ] {
        assert_eq!(
            clear_asker(marker, pane, never),
            ClearAsker::Refused,
            "{marker:?} {pane:?}"
        );
    }
}

#[test]
fn a_read_pane_is_the_human_only_when_it_is_described_and_no_seat() {
    assert_eq!(
        clear_asker(Marker::Resolved, Some("%3"), |pane| {
            assert_eq!(pane, "%3");
            Some(human("work"))
        }),
        ClearAsker::Pane("work".to_owned())
    );
    for read in [
        None,
        Some(PaneRead {
            session: None,
            human: true,
        }),
        Some(PaneRead {
            session: Some("work".to_owned()),
            human: false,
        }),
    ] {
        assert_eq!(
            clear_asker(Marker::Resolved, Some("%3"), |_| read.clone()),
            ClearAsker::Refused
        );
    }
}

#[test]
fn the_refusal_and_the_prompt_name_the_stem_and_the_human_route() {
    let line = clear_refusal("client-muse");
    assert!(!line.contains('\n'));
    assert!(
        line.contains("ae quota clear client-muse") && line.contains("human"),
        "{line}"
    );
    let prompt = clear_prompt("provider-github-copilot");
    assert!(
        prompt.contains("provider-github-copilot") && prompt.ends_with("(y/n)"),
        "{prompt}"
    );
}
