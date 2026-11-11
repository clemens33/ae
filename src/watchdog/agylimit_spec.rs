//! Independent phase-1 acceptance spec; constructed frames, not live limit captures.
use crate::tool::ToolKind;
use crate::watchdog::{Throttle, limit_notice, throttle_class};

const ROW: &str = "⚠ Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 162h20m6s.";
const TEXT: &str = "Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 162h20m6s.";
const ERROR: &str = "Error ID: 00000000-0000-0000-0000-000000000000-000";
const EMPTY: &str = include_str!("../../tests/fixtures/agy-composer/agy-composed-frame-80x24.txt");
const WIDE: &str = include_str!("../../tests/fixtures/agy-composer/agy-composed-frame.txt");

/// Keep the measured composer and put the reported limit block immediately
/// above its upper rule. This changes no frozen fixture bytes on disk.
fn frame(composer: &str, block: &str) -> String {
    let fence = composer
        .lines()
        .find(|row| row.starts_with("────"))
        .expect("a measured upper rule");
    composer.replacen(fence, &format!("{block}\n\n{fence}"), 1)
}

fn limited(composer: &str) -> String {
    frame(composer, &format!("{ROW}\n{ERROR}"))
}

#[test]
fn agylimit_the_reported_row_books_limit_and_quotes_vendor_text() {
    assert_eq!(throttle_class(ROW, "agy"), Some(Throttle::LimitReached));
    assert_eq!(limit_notice(ROW, "agy").as_deref(), Some(TEXT));
    let block = format!("{ROW}\n{ERROR}\n\n");
    let expected = format!("{TEXT} {ERROR}");
    assert_eq!(
        limit_notice(&block, "agy").as_deref(),
        Some(expected.as_str())
    );
    for composer in [EMPTY, WIDE] {
        let capture = limited(composer);
        assert_eq!(
            throttle_class(&capture, "agy"),
            Some(Throttle::LimitReached)
        );
        assert_eq!(
            limit_notice(&capture, "agy").as_deref(),
            Some(expected.as_str())
        );
    }
}

#[test]
fn agylimit_quotes_wrong_columns_and_other_tools_are_not_vendor_rows() {
    for capture in [
        TEXT.to_owned(),
        format!(" {ROW}"),
        format!("  {ROW}"),
        format!("\t{ROW}"),
        format!("> {ROW}"),
        format!("⚠ Note: {TEXT}"),
        format!("⚠ \"{TEXT}\""),
        format!("⚠{TEXT}"),
        format!("⚠\u{fe0f} {TEXT}"),
        "⚠ individual quota reached".to_owned(),
        format!("reported: {ROW}"),
    ] {
        assert_eq!(throttle_class(&capture, "agy"), None, "{capture:?}");
        assert_eq!(limit_notice(&capture, "agy"), None, "{capture:?}");
    }
    for bin in [
        "claude", "codex", "muse", "opencode", "gemini", "grok", "", "Agy",
    ] {
        assert_eq!(throttle_class(ROW, bin), None, "{bin}");
        assert_eq!(limit_notice(ROW, bin), None, "{bin}");
    }
}

#[test]
fn agylimit_no_unmeasured_transient_or_borrowed_banner_is_added() {
    for capture in [
        "⚠ API Error: Overloaded",
        "⚠ unexpected status 429 Too Many Requests",
        "⚠ Rate limit exceeded",
        "⚠ HTTP 503 Service Unavailable",
        "⚠ Usage limit reached",
        "⚠ Your AI credits balance is too low to continue.",
        "■ You’ve hit your usage limit.",
        "  ⎿  You've hit your session limit",
        "◆ Usage limit reached · /upgrade",
        "   ⬝⬝⬝⬝⬝⬝⬝⬝ Free usage exceeded, subscribe to Go",
        "",
        "working normally",
    ] {
        assert_eq!(throttle_class(capture, "agy"), None, "{capture:?}");
    }
}

#[test]
fn agylimit_only_the_head_is_fixed_and_the_newest_vendor_cell_is_quoted() {
    let changed = "⚠ Individual quota reached: server wording can change";
    assert_eq!(throttle_class(changed, "agy"), Some(Throttle::LimitReached));
    assert_eq!(
        limit_notice(changed, "agy").as_deref(),
        Some("Individual quota reached: server wording can change")
    );
    let both = format!("{ROW}\n\n{changed}\n\n");
    assert_eq!(
        limit_notice(&both, "agy").as_deref(),
        Some("Individual quota reached: server wording can change")
    );
    let wrapped = "⚠ Individual quota reached. Please upgrade your subscription to increase your\nlimits. Resets in 162h20m6s.";
    assert_eq!(limit_notice(wrapped, "agy").as_deref(), Some(TEXT));
}

#[test]
fn agylimit_twelve_row_window_is_measured_from_last_ink_not_trailing_blanks() {
    for (under, books) in [(0, true), (11, true), (12, false), (20, false)] {
        let capture = format!("{ROW}\n{}\n\n\n", "ink\n".repeat(under));
        assert_eq!(
            throttle_class(&capture, "agy"),
            books.then_some(Throttle::LimitReached),
            "{under} rows below"
        );
        assert_eq!(
            limit_notice(&capture, "agy").is_some(),
            books,
            "{under} rows below"
        );
    }
}

#[test]
fn agylimit_other_tools_keep_limit_notice_and_transient_contracts() {
    for (bin, row, text) in [
        (
            "claude",
            "  ⎿  You've hit your session limit · resets 3pm",
            "You've hit your session limit ? resets 3pm",
        ),
        (
            "codex",
            "■ You’ve hit your usage limit.",
            "You?ve hit your usage limit.",
        ),
        (
            "muse",
            "◆ Usage limit reached · /upgrade",
            "Usage limit reached ? /upgrade",
        ),
        (
            "opencode",
            "   ⬝⬝⬝⬝⬝⬝⬝⬝ Free usage exceeded, subscribe to Go",
            "Free usage exceeded, subscribe to Go",
        ),
    ] {
        assert_eq!(
            throttle_class(row, bin),
            Some(Throttle::LimitReached),
            "{bin}"
        );
        assert_eq!(limit_notice(row, bin).as_deref(), Some(text), "{bin}");
    }
    for (bin, row) in [
        ("claude", "⏺ API Error: Overloaded"),
        ("codex", "■ unexpected status 429 Too Many Requests"),
    ] {
        assert_eq!(throttle_class(row, bin), Some(Throttle::Throttled), "{bin}");
        assert_eq!(limit_notice(row, bin), None, "{bin}");
    }
}

fn observation(capture: &str) -> crate::watchdog_daemon::Observation {
    use crate::watchdog_daemon::{HarnessObservation, Observation};
    Observation {
        now_epoch: 1_700_000_000,
        hash: 7,
        harness: HarnessObservation {
            frame: crate::harness_state::classify(capture, ToolKind::Agy),
            human_draft: false,
            durable_stale: false,
            declaration: None,
        },
        identity: 1,
        is_dead: false,
        throttle: throttle_class(capture, "agy"),
        limit_notice: limit_notice(capture, "agy"),
        limit_reset: None,
        limit_woke_journal: None,
        auto_continue: false,
        capture_ok: true,
        human_prompt: None,
        throttle_quota: None,
        quiet: None,
        ended_wait: None,
        done_progress: crate::watchdog::DoneProgress::None,
        wait_progress: crate::watchdog::WaitProgress::None,
        wait_rung: 0,
        wait_ends_elsewhere: false,
        target_moved: None,
        descendancy: crate::procs::Descendancy::Present,
        launch_attempt: crate::tmux::Evidence::Silent,
        outstanding_alert: None,
        unreachable: false,
        failed_elsewhere: false,
        prompt_named: false,
        start: None,
        last_actor_event_age_secs: 0,
        declared_age_secs: 0,
        suspended: crate::watchdog::Suspensions::default(),
        sweep: None,
        own_work: crate::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}

#[test]
fn agylimit_classifier_drives_one_latch_notice_and_the_existing_limit_route() {
    use crate::watchdog_daemon::{Effect, Knobs, PaneState, Verdict, account};
    let capture = limited(EMPTY);
    let seen = observation(&capture);
    let first = account(&PaneState::default(), &seen, &Knobs::default());
    assert_eq!(first.verdict, Verdict::Limit);
    assert_eq!(first.next.limit_since, Some(seen.now_epoch));
    assert_eq!(
        first.next.limit_reset, None,
        "relative vendor timer has no proven draw time"
    );
    let limits: Vec<_> = first
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit {
                action: "limit",
                summary,
            } => Some(summary),
            _ => None,
        })
        .collect();
    assert_eq!(limits.len(), 1);
    assert!(limits[0].contains(TEXT), "journal quotes the vendor row");
    assert_eq!(
        first
            .effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Notify(line) if line.contains(TEXT)))
            .count(),
        1
    );
    let again = account(&first.next, &seen, &Knobs::default());
    assert_eq!(again.verdict, Verdict::Limit);
    assert_eq!(again.next.limit_reset, None);
    assert!(!again.effects.iter().any(|effect| matches!(
        effect,
        Effect::Emit {
            action: "limit",
            ..
        } | Effect::Notify(_)
            | Effect::Nudge
    )));

    let mut failed = seen.clone();
    failed.capture_ok = false;
    failed.throttle = None;
    failed.limit_notice = None;
    assert_eq!(
        account(&first.next, &failed, &Knobs::default()).verdict,
        Verdict::Limit
    );
    let cleared = account(&first.next, &observation(EMPTY), &Knobs::default());
    assert_eq!(
        cleared.next.limit_since, None,
        "successful no-limit read releases"
    );
}

#[test]
fn agylimit_existing_limit_route_accepts_an_unmodelled_limited_seat() {
    // The event written by this branch is the existing route's input. Keep
    // identity, timestamp and target fields in the journal's actual grammar.
    let event = crate::events::Event::parse_line(r#"{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"limit","target":"agyseat","target_slot":"spawned.2","target_session":"sample"}"#).expect("limit event");
    let episode = crate::autoreseat::episode(&[event], "sample", "spawned.2", "agyseat")
        .expect("limit episode");
    let pane = crate::autoreseat::Pane {
        frame: crate::autoreseat::Frame::Unproven,
        human_prompt: false,
        client_input: None,
    };
    assert_eq!(
        crate::autoreseat::decide(Some(&episode), 0, &pane, 1_700_000_001),
        crate::autoreseat::Decision::Attempt
    );
}
