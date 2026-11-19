//! FROZEN SPEC (vendorwait; issue #126, ae-feedback 64): Codex's service-wait
//! menu is read BY NAME as a busy seat. Install byte-identically as
//! `tests/it/vendorwait_spec.rs`, with `mod vendorwait_spec;` in
//! `tests/it/main.rs`.
//!
//! Oracle: the vendor's own insta snapshots of the menu
//! (`codex-rs/tui/src/chatwidget/snapshots/codex_tui__chatwidget__tests__safety_buffering_*.snap`
//! at tags `rust-v0.152.1` and `rust-v0.170.0`), rendered by the vendor at 80
//! columns. The rows under the transcript row are those snapshots verbatim; the
//! 200-column fixture and the styled `.esc` twins are RECONSTRUCTED from the
//! same strings and the vendor's rendering code (see the fixture READMEs).
//! The menu is one view in both revisions: its footer says no action is
//! required and Codex dismisses it itself when the response streams, so it
//! is a wait, never a question for the human.

use ae::deliver::region::{self, Occupancy};
use ae::harness_state::{self, HarnessState};
use ae::tool::{InputModel, ToolKind};

/// The words `ae list` prints after `observed:` for a seat in the wait.
const NAMED: &str = "busy (codex service wait)";

/// Every vendor revision and layout of the menu, plain (the watchdog's capture).
const WAITS: [(&str, &str); 4] = [
    (
        "0.170.0 at 80 columns",
        include_str!("../fixtures/harness-state/codex-service-wait-0.170.0-80w.txt"),
    ),
    (
        "0.152.1 at 80 columns",
        include_str!("../fixtures/harness-state/codex-service-wait-0.152.1-80w.txt"),
    ),
    (
        "0.170.0 without the retry option",
        include_str!("../fixtures/harness-state/codex-service-wait-noretry-0.170.0-80w.txt"),
    ),
    (
        "0.170.0 at 200 columns",
        include_str!("../fixtures/harness-state/codex-service-wait-0.170.0-200w.txt"),
    ),
];

/// The same menus as delivery captures them (`capture-pane -e`).
const STYLED_WAITS: [(&str, &str); 3] = [
    (
        "0.170.0",
        include_str!("../fixtures/codex-composer/codex-service-wait-0.170.0-80w.esc"),
    ),
    (
        "0.152.1",
        include_str!("../fixtures/codex-composer/codex-service-wait-0.152.1-80w.esc"),
    ),
    (
        "0.170.0 without the retry option",
        include_str!("../fixtures/codex-composer/codex-service-wait-noretry-0.170.0-80w.esc"),
    ),
];

const QUOTED: &str =
    include_str!("../fixtures/harness-state/codex-idle-quoting-service-wait-0.156.1-200w.txt");
const QUOTED_STYLED: &str =
    include_str!("../fixtures/codex-composer/codex-idle-quoting-service-wait-0.156.1-200w.esc");
const IDLE: &str = include_str!("../fixtures/harness-state/codex-idle-0.156.1-200x40.txt");

/// RED on main (the frame reads `unknown`): the wait is busy on the protocol
/// word every machine reader keeps, and named on the human word.
#[test]
fn the_codex_service_wait_reads_busy_by_name_in_every_vendor_revision() {
    for (name, frame) in WAITS {
        let state = harness_state::classify(frame, ToolKind::Codex);
        assert_eq!(state.as_str(), "busy", "{name}: the protocol word");
        assert_eq!(state.human(), NAMED, "{name}: the ae list words");
        // A pane taller than its transcript draws blank rows under the menu.
        let padded = format!("{frame}\n\n\n");
        assert_eq!(
            harness_state::classify(&padded, ToolKind::Codex).human(),
            NAMED,
            "{name}: blank rows below the menu"
        );
        assert!(
            !harness_state::has_human_draft(frame, ToolKind::Codex),
            "{name}: the menu is no human draft"
        );
    }
}

/// GREEN on main, a guard: the wait is never a human-only prompt, so nothing
/// names it needs-you and `auto_trust` never answers or dismisses it.
#[test]
fn the_codex_service_wait_is_no_human_only_prompt() {
    for (name, frame) in WAITS {
        assert_eq!(
            ae::watchdog::human_prompt_class(frame, "codex"),
            None,
            "{name}"
        );
    }
}

/// RED on main (the selected option row reads as 57 content cells on 3 rows,
/// ae-feedback 64): the menu proves NO composer, so neither the pre-paste
/// gate nor the submit check can read it as a draft or a staged paste.
#[test]
fn the_styled_service_wait_proves_no_composer() {
    for (name, frame) in STYLED_WAITS {
        let read = region::read(frame, InputModel::StyleDelimited);
        assert_eq!(read.occupancy, Occupancy::Unreadable, "{name}: {read:?}");
        assert_eq!((read.cells, read.rows), (0, 0), "{name}");
    }
}

/// GREEN on main, a control: the vendor's words QUOTED in a transcript above
/// codex's live empty composer are transcript, and the seat is idle.
#[test]
fn the_vendor_words_quoted_above_a_live_composer_stay_idle() {
    assert_eq!(
        harness_state::classify(QUOTED, ToolKind::Codex),
        HarnessState::Idle
    );
    assert!(!harness_state::has_human_draft(QUOTED, ToolKind::Codex));
    assert_eq!(
        region::read(QUOTED_STYLED, InputModel::StyleDelimited).occupancy,
        Occupancy::Idle
    );
}

/// GREEN on main, a control: a human draft that quotes the vendor's title is
/// a draft in a live composer, never the wait.
#[test]
fn a_draft_quoting_the_vendor_title_is_a_draft() {
    let draft = IDLE.replace(
        "› Ask Codex to do anything",
        "› Giving this request a little extra thought",
    );
    assert_ne!(draft, IDLE, "setup: the placeholder row was replaced");
    assert!(harness_state::has_human_draft(&draft, ToolKind::Codex));
    assert_ne!(
        harness_state::classify(&draft, ToolKind::Codex).human(),
        NAMED
    );
}

/// GREEN on main, a control: today's codex frames keep today's words.
#[test]
fn the_plain_codex_frames_keep_their_verdicts() {
    let busy = harness_state::classify(
        include_str!("../fixtures/harness-state/codex-busy-280x40.txt"),
        ToolKind::Codex,
    );
    assert_eq!((busy, busy.human()), (HarnessState::Busy, "busy"));
    let idle = harness_state::classify(IDLE, ToolKind::Codex);
    assert_eq!((idle, idle.human()), (HarnessState::Idle, "input ready"));
    let modal = harness_state::classify(
        include_str!("../fixtures/harness-state/codex-modal-112x40.txt"),
        ToolKind::Codex,
    );
    assert_eq!(modal, HarnessState::Unknown);
    let carried = harness_state::observed_from_option("busy");
    assert_eq!((carried, carried.human()), (HarnessState::Busy, "busy"));
}
