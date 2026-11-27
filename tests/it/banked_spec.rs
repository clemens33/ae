//! Frozen banked-resets acceptance: dated `manual_resets=` declarations in `ae quota`.
//!
//! INSTALL (driver, byte-identical): copy to `tests/it/banked_spec.rs` and add
//! `mod banked_spec;` to `tests/it/main.rs` in its sorted place.
//! Oracle: `.local/brief-banked.md` rulings 1-3, the driver's accepted tie and
//! boundary rules (ae-20261127T093221Z-6aa6581a thread), and hand-computed
//! epoch arithmetic below. Never the code under test.
//!
//! Existing API only (`ae::quota::run` with `Inputs.now`): on the base commit
//! this file COMPILES, and the dated-declaration assertions fail at runtime.
//!
//! Clock: `NOW` = `1_788_858_600` = `2026-09-08T09:10:00Z`. Every timestamp below
//! is that instant plus a stated whole number of seconds, written at minute
//! precision with an explicit offset.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures build and inspect real directories under an owned scratch root"
)]

use std::path::Path;

use super::cli::OwnedScratch;

const NOW: i64 = 1_788_858_600; // 2026-09-08T09:10:00Z
const ID: &str = "01a08046-1974-7352-ade3-81a786200795";
/// The weekly window's own reset: NOW + 6 d.
const WEEKLY_RESET: i64 = 1_789_377_000; // 2026-09-14T09:10:00Z

/// NOW - 1 h.
const PAST: &str = "2026-09-08T08:10+00:00";
/// NOW exactly: expired at the observation instant.
const AT_NOW: &str = "2026-09-08T09:10+00:00";
/// NOW - 1 min, spelled at +01:00; a reader that drops the offset sees NOW + 59 min.
const PAST_IN_PLUS_ONE: &str = "2026-09-08T10:09+01:00";
/// NOW + 1 min, spelled at +02:00; a reader that drops the offset sees NOW + 2 h 1 min.
const FUTURE_IN_PLUS_TWO: &str = "2026-09-08T11:11+02:00";
/// NOW + 24 h.
const IN_24H: &str = "2026-09-09T09:10+00:00";
/// NOW + 36 h.
const IN_36H: &str = "2026-09-09T21:10+00:00";
/// NOW + 3 d.
const IN_3D: &str = "2026-09-11T09:10+00:00";
/// NOW + 4 d 14 h, spelled at +01:00.
const IN_4D14H: &str = "2026-09-13T00:10+01:00";
/// NOW + 20 d: after the weekly reset.
const IN_20D: &str = "2026-09-28T09:10+00:00";
/// The owner's three real expiry dates; all are after NOW.
const OWNER: &str = "2026-12-01T23:22+01:00,2026-12-12T23:17+01:00,2026-12-26T06:48+01:00";

/// One Codex weekly `codex` bucket reading.
fn record(timestamp: &str, used: f64) -> String {
    format!(
        r#"{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"limit_id":"codex","limit_name":null,"primary":{{"used_percent":{used:.1},"window_minutes":10080,"resets_at":{WEEKLY_RESET}}},"secondary":null,"plan_type":"pro"}}}}}}
"#
    )
}

/// The default rollout: one reading at NOW - 1 min, too short a history for a pace.
fn one_reading(used: f64) -> String {
    record("2026-09-08T09:09:00.000Z", used)
}

/// A hermetic rig: one client `cx` (no config home), one profile, one fleet seat
/// whose rollout is `rollout`.
fn rig(tag: &str, declaration: &str, rollout: &str) -> OwnedScratch {
    let root = OwnedScratch::root("banked", tag);
    let path = root.path();
    let day = path.join(".codex/sessions/2026/09/08");
    std::fs::create_dir_all(&day).expect("rollout day");
    std::fs::create_dir_all(path.join("sessions/session")).expect("session directory");
    let tail = if declaration.is_empty() {
        String::new()
    } else {
        format!(" manual_resets={declaration}")
    };
    std::fs::write(
        path.join("config"),
        format!("[clients]\ncx = codex{tail}\n[profiles]\nastrax = cx --model astra\n"),
    )
    .expect("config");
    std::fs::write(
        path.join("sessions/session/meta"),
        format!(
            "schema=2\nseat.main=lead\nprofile.main=astrax\nharness_session.main={ID}\nagent_bin.main=codex\n"
        ),
    )
    .expect("meta");
    std::fs::write(
        day.join(format!("rollout-2026-09-08T09-00-00-{ID}.jsonl")),
        rollout,
    )
    .expect("rollout");
    root
}

fn run_quota(root: &Path) -> String {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(root),
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut stderr,
    )
    .expect("quota runs");
    assert_eq!(code, 0);
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    String::from_utf8(stdout).expect("UTF-8 table")
}

/// The trimmed cell under `header` on the first `codex (pro)` row, sliced by the
/// header line's own column positions, so a new column anywhere keeps it exact.
fn cell(text: &str, header: &str) -> String {
    let head = text
        .lines()
        .find(|line| line.starts_with("PROFILES"))
        .unwrap_or_else(|| panic!("a table header: {text}"));
    let start = head
        .find(header)
        .unwrap_or_else(|| panic!("a {header} column: {text}"));
    // The column runs to the first letter of the next header word.
    let end = head[start + header.len()..]
        .find(|c: char| !c.is_whitespace())
        .map_or(head.len(), |next| start + header.len() + next);
    let row = text
        .lines()
        .find(|line| line.contains("codex (pro)"))
        .unwrap_or_else(|| panic!("a codex row: {text}"));
    let chars: Vec<char> = row.chars().collect();
    let width = head[..start].chars().count();
    let span = head[start..end].chars().count();
    chars
        .iter()
        .skip(width)
        .take(span)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The lines that state a refused declaration.
fn refusals(text: &str) -> usize {
    text.lines()
        .filter(|line| line.contains("manual_resets=") && line.contains("ignored"))
        .count()
}

/// The text right after the `· next ` marker, spaces removed, so `4d14h` and
/// `4d 14h` read alike.
fn next_after(text: &str, count: u8) -> Option<String> {
    let marker = format!("{count} · next ");
    let at = text.find(&marker)?;
    Some(
        text[at + marker.len()..]
            .chars()
            .take(10)
            .filter(|c| *c != ' ')
            .collect(),
    )
}

fn advisories(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.contains("banked reset expires"))
        .collect()
}

#[test]
fn banked_dates_count_only_resets_that_are_unexpired_at_the_observation_instant() {
    let cases = [
        ("undated", "1".to_owned(), "45% x1"),
        ("owner", OWNER.to_owned(), "22.5% x3"),
        ("two-future", format!("{IN_3D},{IN_20D}"), "30% x2"),
        ("one-past", format!("{PAST},{IN_3D}"), "45% x1"),
        ("at-now", format!("{AT_NOW},{IN_3D}"), "45% x1"),
        (
            "offset-past",
            format!("{PAST_IN_PLUS_ONE},{IN_3D}"),
            "45% x1",
        ),
        (
            "offset-future",
            format!("{FUTURE_IN_PLUS_TWO},{IN_3D}"),
            "30% x2",
        ),
        ("all-expired", format!("{PAST},{AT_NOW}"), "90%"),
        ("nine", [IN_3D; 9].join(","), "9% x9"),
    ];
    for (tag, declaration, expected) in cases {
        let root = rig(tag, &declaration, &one_reading(90.0));
        let text = run_quota(root.path());
        assert_eq!(
            cell(&text, "EFFECTIVE"),
            expected,
            "{tag}: manual_resets={declaration} judges the window over the unexpired count: {text}"
        );
        assert_eq!(refusals(&text), 0, "{tag}: a usable declaration: {text}");
    }
}

#[test]
fn an_unusable_dated_declaration_is_refused_with_one_note_and_derives_nothing() {
    let ten = [IN_3D; 10].join(",");
    let unstated: &[&str] = &[];
    let cases = [
        ("no-offset", "2026-12-01T23:22".to_owned(), &["offset"][..]),
        (
            "one-without-offset",
            "2026-12-01T23:22+01:00,2026-12-12T23:17".to_owned(),
            &["offset"][..],
        ),
        ("bad-month", "2026-13-01T23:22+01:00".to_owned(), unstated),
        ("bad-day", "2026-02-30T10:00+01:00".to_owned(), unstated),
        ("bad-hour", "2026-12-01T25:00+01:00".to_owned(), unstated),
        ("bad-offset", "2026-12-01T23:22+25:00".to_owned(), unstated),
        ("empty-item", format!("{IN_3D},,{IN_20D}"), unstated),
        ("trailing-comma", format!("{IN_3D},"), unstated),
        ("count-and-date", format!("2,{IN_3D}"), unstated),
        ("ten", ten, &["above 9", "more than 9"][..]),
    ];
    for (tag, declaration, reasons) in cases {
        let root = rig(tag, &declaration, &one_reading(90.0));
        let text = run_quota(root.path());
        assert_eq!(
            refusals(&text),
            1,
            "{tag}: manual_resets={declaration} is refused in exactly one note: {text}"
        );
        if !reasons.is_empty() {
            let note = text
                .lines()
                .find(|line| line.contains("manual_resets=") && line.contains("ignored"))
                .unwrap_or_default();
            assert!(
                reasons.iter().any(|reason| note.contains(reason)),
                "{tag}: the note says why ({reasons:?}): {note}"
            );
        }
        assert_eq!(
            cell(&text, "EFFECTIVE"),
            "-",
            "{tag}: a refused declaration derives nothing: {text}"
        );
        assert!(
            !text.contains("· next"),
            "{tag}: a refused declaration shows no expiry: {text}"
        );
    }
}

#[test]
fn a_dated_declaration_shows_its_count_and_next_expiry_once_per_scope() {
    let root = rig(
        "shown",
        &format!("{PAST},{IN_20D},{IN_4D14H}"),
        &one_reading(10.0),
    );
    let text = run_quota(root.path());
    assert_eq!(
        text.matches("· next").count(),
        1,
        "one cell for the one scope, not one per window row: {text}"
    );
    let next = next_after(&text, 2)
        .unwrap_or_else(|| panic!("`2 · next ` for two unexpired dates: {text}"));
    assert!(
        next.starts_with("4d14h"),
        "the nearest unexpired date, in either written order, is 4 d 14 h away: {next:?} in {text}"
    );
    assert!(
        text.contains("edit after use"),
        "ae never consumes a banked reset, and says so: {text}"
    );

    for (tag, declaration) in [("undated", "2"), ("none", "")] {
        let root = rig(tag, declaration, &one_reading(10.0));
        let text = run_quota(root.path());
        assert!(
            !text.contains("· next"),
            "{tag}: an undated or absent declaration has no expiry to show: {text}"
        );
    }
}

#[test]
fn an_expiry_before_the_weekly_reset_at_unknown_pace_advises_spending_the_scope() {
    // One reading: PACE is unknown, so the cutoff is the weekly reset (NOW + 6 d).
    let on = [
        ("before-reset", format!("{IN_3D},{IN_20D}")),
        ("two-before-reset", format!("{IN_36H},{IN_3D}")),
    ];
    for (tag, declaration) in on {
        let root = rig(tag, &declaration, &one_reading(10.0));
        let text = run_quota(root.path());
        assert_eq!(
            cell(&text, "PACE"),
            "unknown",
            "{tag}: fixture has no pace: {text}"
        );
        let lines = advisories(&text);
        assert_eq!(
            lines.len(),
            1,
            "{tag}: one advisory line per scope, naming its next expiry: {text}"
        );
        for word in ["unused", "spend", "harder", "cx"] {
            assert!(
                lines[0].contains(word),
                "{tag}: the advisory names `{word}`: {}",
                lines[0]
            );
        }
    }
    let off = [
        ("after-reset", IN_20D.to_owned()),
        ("undated", "2".to_owned()),
        ("all-expired", format!("{PAST},{AT_NOW}")),
        ("none", String::new()),
    ];
    for (tag, declaration) in off {
        let root = rig(tag, &declaration, &one_reading(10.0));
        let text = run_quota(root.path());
        assert!(
            advisories(&text).is_empty(),
            "{tag}: nothing expires unused before the window turns over: {text}"
        );
    }
}

#[test]
fn a_known_pace_decides_whether_a_banked_reset_would_expire_unused() {
    // Two readings 5 h 59 min apart in one weekly window instance, one declared
    // reset, so the judged percentage spreads the usage over two windows. The
    // advisory compares the next expiry with the moment the CURRENT weekly
    // window runs out (brief ruling 3), never with the PACE ETA over all
    // declared capacity, which already counts the banked reset as spendable.
    //
    // fast: raw 10% -> 70%, about 10 %/h: the window runs out in about 3 h
    // (all capacity about 13 h). The NOW + 3 d expiry comes later, so the reset
    // will be used: NO advisory, although the expiry is before the weekly reset
    // (the unknown-pace cutoff).
    //
    // middle: raw 10% -> 40%, about 5 %/h: the window runs out in about 12 h,
    // all capacity in about 32 h. The NOW + 24 h expiry falls between them: the
    // window runs out first and the reset will be used, so NO advisory. A cutoff
    // read from the all-capacity ETA would wrongly advise here.
    //
    // slow: raw 10% -> 20%, about 1.7 %/h: the window runs out in about 48 h
    // (all capacity about 107 h). The NOW + 36 h expiry comes first: ONE advisory.
    let rollout = |later: f64| {
        record("2026-09-08T03:10:00.000Z", 10.0) + &record("2026-09-08T09:09:00.000Z", later)
    };
    for (tag, later, expiry, advised) in [
        ("fast", 70.0, IN_3D, false),
        ("middle", 40.0, IN_24H, false),
        ("slow", 20.0, IN_36H, true),
    ] {
        let root = rig(tag, expiry, &rollout(later));
        let text = run_quota(root.path());
        let pace = cell(&text, "PACE");
        assert!(
            pace.contains("%/h") && pace.contains('~'),
            "{tag}: the fixture states a rate and an ETA: {pace:?} in {text}"
        );
        assert_eq!(
            advisories(&text).len(),
            usize::from(advised),
            "{tag}: the advisory follows the current window's run-out, not only the reset: {text}"
        );
    }
}

#[test]
fn the_smallest_active_count_wins_across_labels_of_one_config_home() {
    // (a, b, effective at 90% used, the shown count, whether an expiry shows)
    let cases = [
        // Active, not declared, counts compare: a holds three dates but one active.
        (
            format!("{PAST},{AT_NOW},{IN_4D14H}"),
            "2".to_owned(),
            "45% x1",
            1,
            true,
        ),
        (
            "2".to_owned(),
            format!("{PAST},{AT_NOW},{IN_4D14H}"),
            "45% x1",
            1,
            true,
        ),
        // Undated smaller wins over a dated larger.
        (
            format!("{IN_3D},{IN_4D14H}"),
            "1".to_owned(),
            "45% x1",
            1,
            false,
        ),
        // Equal active counts: the dated declaration wins, so its expiry shows.
        (
            format!("{PAST},{IN_4D14H}"),
            "1".to_owned(),
            "45% x1",
            1,
            true,
        ),
        (
            "1".to_owned(),
            format!("{PAST},{IN_4D14H}"),
            "45% x1",
            1,
            true,
        ),
    ];
    for (index, (a, b, effective, count, dated)) in cases.into_iter().enumerate() {
        let root = OwnedScratch::root("banked", &format!("smallest-{index}"));
        let path = root.path();
        let shared = path.join(".codex-shared/sessions/2026/09/08");
        std::fs::create_dir_all(&shared).expect("shared rollout day");
        std::fs::create_dir_all(path.join("sessions/session")).expect("session directory");
        std::fs::write(
            shared.join(format!("rollout-2026-09-08T09-00-00-{ID}.jsonl")),
            one_reading(90.0),
        )
        .expect("shared rollout");
        std::fs::write(
            path.join("config"),
            format!(
                "[clients]\na = codex config_home=$HOME/.codex-shared manual_resets={a}\nb = codex config_home=$HOME/.codex-shared manual_resets={b}\n[profiles]\nax = a --model x\nbx = b --model y\n"
            ),
        )
        .expect("conflicting config");
        std::fs::write(
            path.join("sessions/session/meta"),
            format!(
                "schema=2\nseat.main=lead\nprofile.main=ax\nharness_session.main={ID}\nagent_bin.main=codex\n"
            ),
        )
        .expect("session meta");
        let text = run_quota(path);
        assert_eq!(
            cell(&text, "EFFECTIVE"),
            effective,
            "case {index}: a={a} b={b} judges by the smallest active count: {text}"
        );
        let next = next_after(&text, count);
        if dated {
            assert!(
                next.as_deref()
                    .is_some_and(|next| next.starts_with("4d14h")),
                "case {index}: the winning dated declaration shows `{count} · next 4d14h`: {text}"
            );
        } else {
            assert!(
                !text.contains("· next"),
                "case {index}: the winning undated count shows no expiry: {text}"
            );
        }
        assert_eq!(refusals(&text), 0, "case {index}: both are usable: {text}");
    }
}

#[test]
fn an_expiry_at_the_weekly_reset_itself_gets_no_advisory() {
    // r7: the cutoff is strict. A reset expiring at the instant the window
    // turns over is replaced by the vendor reset, so nothing expires unused.
    // One minute earlier the same declaration advises, so the pin is not vacuous.
    let at_reset = "2026-09-14T09:10+00:00"; // WEEKLY_RESET, NOW + 6 d
    let minute_before = "2026-09-14T09:09+00:00"; // WEEKLY_RESET - 60 s
    let root = rig("at-weekly-reset", at_reset, &one_reading(10.0));
    let text = run_quota(root.path());
    assert_eq!(
        cell(&text, "PACE"),
        "unknown",
        "fixture has no pace: {text}"
    );
    assert!(
        advisories(&text).is_empty(),
        "an expiry at the weekly reset itself is not unused: {text}"
    );
    let root = rig("before-weekly-reset", minute_before, &one_reading(10.0));
    let text = run_quota(root.path());
    assert_eq!(
        advisories(&text).len(),
        1,
        "one minute before the weekly reset it advises: {text}"
    );
}

#[test]
fn a_known_pace_cutoff_is_strict_at_the_raw_runout_instant() {
    // r7: one declared reset; raw 20% at NOW - 4 h and 60% at NOW in one weekly
    // window instance: 10 %/h raw, so 40% raw headroom runs out in exactly 4 h,
    // at 13:10Z (the judged PACE is 5 %/h over both windows). A reset expiring at
    // that instant is not unused; one minute earlier it is. Both are before the
    // weekly reset, so only the run-out decides.
    let rollout =
        record("2026-09-08T05:10:00.000Z", 20.0) + &record("2026-09-08T09:10:00.000Z", 60.0);
    for (tag, expiry, advised) in [
        ("at-runout", "2026-09-08T13:10+00:00", false),
        ("minute-before-runout", "2026-09-08T13:09+00:00", true),
    ] {
        let root = rig(tag, expiry, &rollout);
        let text = run_quota(root.path());
        let pace = cell(&text, "PACE");
        assert!(
            pace.contains("%/h"),
            "{tag}: the fixture measures a rate: {pace:?} in {text}"
        );
        assert_eq!(
            advisories(&text).len(),
            usize::from(advised),
            "{tag}: the raw run-out cutoff is strict: {text}"
        );
    }
}
