//! Independent acceptance: chat prints action changes, not clock noise.
//! Oracle: chatquiet brief P1–P4 and its settled visibility rulings.

#![allow(
    clippy::expect_used,
    reason = "synthetic fixture setup fails loudly inside test-owned scratch"
)]

use ae::attention::Reason;
use ae::console::needs::{Cause, Row, STALE_SECS, SeatRef, Section, Source, Verdict};
use ae::console::view::Printed;
use ae::time::Timestamp;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const START: i64 = 1_790_748_000;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_epoch(START + seconds)
}

fn reason(name: &str, why: Reason) -> Row {
    Row {
        seat: SeatRef {
            slot: format!("spawned.{name}"),
            name: name.to_owned(),
        },
        lead_pair: false,
        verdict: Verdict::Reason(why),
        source: Source::Declaration,
        stale: None,
        since_micros: Some((START - 60) * 1_000_000),
        detail: format!("ACTION-{name}"),
        record: Some(1),
    }
}

fn unknown(name: &str, cause: Cause) -> Row {
    let mut row = reason(name, Reason::Blocked);
    row.verdict = Verdict::Unknown(cause);
    row.source = Source::Unverified;
    row.since_micros = None;
    row.detail.clear();
    row.record = None;
    row
}

fn show(printed: &mut Printed, rows: &[Row], seconds: i64) -> String {
    printed.needs(
        &Ok(Section {
            rows: rows.to_vec(),
        }),
        None,
        at(seconds),
    )
}

fn stale_rows(rows: &[Row], last: i64) -> Vec<Row> {
    let cause = Cause::WatchdogStale {
        last_micros: (START + last) * 1_000_000,
    };
    let mut rows = rows.to_vec();
    for row in &mut rows {
        row.stale = Some(cause);
    }
    rows.push(unknown("idle", cause));
    rows
}

fn single_silence_notice(text: &str) {
    assert_eq!(text.lines().count(), 1, "one dim episode notice: {text:?}");
    assert!(
        text.contains("watchdog") && text.contains("silent"),
        "silence stays visible: {text}"
    );
    assert!(
        !text.contains("-- needs you:") && !text.contains("ACTION-"),
        "silence alone must not replay action section: {text}"
    );
    assert!(!text.contains('\x1b'), "pipe output stays plain");
}

#[test]
fn quiet_header_counts_actionable_seats_and_uncertainty_has_its_own_count() {
    let rows = vec![
        reason("lead", Reason::WaitingUser),
        reason("scout", Reason::Blocked),
        unknown("one", Cause::WatchdogOff),
        unknown("two", Cause::WatchdogOff),
        unknown("three", Cause::WatchdogOff),
    ];
    let text = show(&mut Printed::default(), &rows, 0);
    let header = text
        .lines()
        .find(|line| line.starts_with("-- needs you:"))
        .unwrap_or("");
    assert!(
        header.starts_with("-- needs you: 2 seats"),
        "only real verdicts count as human work: {text}"
    );
    let doubt = text
        .lines()
        .find(|line| line.contains("unverified"))
        .unwrap_or("");
    assert!(
        doubt.contains("3 seats")
            && doubt.contains("one")
            && doubt.contains("two")
            && doubt.contains("three"),
        "all uncertain seats remain visible: {text}"
    );
}

#[test]
fn quiet_unknown_only_snapshot_never_claims_human_work() {
    let rows = [unknown("idle", Cause::WatchdogOff)];
    let text = show(&mut Printed::default(), &rows, 0);
    assert!(text.contains("unverified") && text.contains("idle") && text.contains("watchdog off"));
    for header in text
        .lines()
        .filter(|line| line.starts_with("-- needs you:"))
    {
        assert!(
            header.contains("0 seats") || header.contains("nothing standing"),
            "unknown-only snapshot never claims positive human work: {text}"
        );
    }
}

#[test]
fn quiet_record_position_and_since_stamp_changes_do_not_replay_actions() {
    let mut printed = Printed::default();
    let mut rows = vec![reason("lead", Reason::WaitingUser)];
    assert!(show(&mut printed, &rows, 0).contains("ACTION-lead"));
    rows[0].since_micros = Some((START - 30) * 1_000_000);
    rows[0].record = Some(999);
    assert!(
        show(&mut printed, &rows, 1).is_empty(),
        "journal positions and since stamps are not new human work"
    );
}

#[test]
fn quiet_age_sort_order_does_not_replay_the_same_action_set() {
    let mut printed = Printed::default();
    let mut rows = vec![
        reason("one", Reason::Blocked),
        reason("two", Reason::Blocked),
    ];
    assert!(!show(&mut printed, &rows, 0).is_empty());
    rows.reverse();
    assert!(
        show(&mut printed, &rows, 1).is_empty(),
        "same seats and verdicts survive age-order changes"
    );
}

#[test]
fn quiet_silence_onset_is_one_notice_without_replaying_actions() {
    let mut printed = Printed::default();
    let rows = [reason("lead", Reason::WaitingUser)];
    assert!(show(&mut printed, &rows, 0).contains("ACTION-lead"));
    let text = show(&mut printed, &stale_rows(&rows, -100), 1);
    single_silence_notice(&text);
    assert!(text.contains("idle"), "unverified seat still named: {text}");
}

#[test]
fn quiet_last_beat_changes_and_recovery_do_not_replay_actions() {
    let mut printed = Printed::default();
    let rows = [reason("lead", Reason::WaitingUser)];
    let _ = show(&mut printed, &rows, 0);
    let _ = show(&mut printed, &stale_rows(&rows, -100), 1);
    assert!(
        show(&mut printed, &stale_rows(&rows, -90), 2).is_empty(),
        "last heartbeat stamp is not a new silence class"
    );
    assert!(
        show(&mut printed, &rows, 3).is_empty(),
        "watchdog recovery alone has no human action"
    );
    single_silence_notice(&show(&mut printed, &stale_rows(&rows, -80), 4));
}

#[test]
fn quiet_a_real_reason_change_during_silence_prints_current_actions() {
    let mut printed = Printed::default();
    let rows = [reason("lead", Reason::WaitingUser)];
    let _ = show(&mut printed, &stale_rows(&rows, -100), 0);
    let changed = [reason("lead", Reason::Dead)];
    let text = show(&mut printed, &stale_rows(&changed, -99), 1);
    assert!(
        text.contains("-- needs you: 1 seat")
            && text.contains("dead")
            && text.contains("ACTION-lead"),
        "changed verdict remains actionable during doubt: {text}"
    );
}

#[test]
fn quiet_proof_detail_is_quiet_but_changed_source_shows_new_action() {
    let mut printed = Printed::default();
    let mut rows = vec![reason("lead", Reason::Blocked)];
    let _ = show(&mut printed, &rows, 0);
    rows[0].detail = "NEW-DECISION".to_owned();
    assert!(
        show(&mut printed, &rows, 1).is_empty(),
        "fresh proof for same verdict is not new human work"
    );
    rows[0].source = Source::Alert {
        action: "human-prompt".to_owned(),
    };
    assert!(show(&mut printed, &rows, 2).contains("human prompt"));
    rows[0].source = Source::Alert {
        action: "limit".to_owned(),
    };
    assert!(
        show(&mut printed, &rows, 3).contains("limit"),
        "changed alert action is a changed source even with same reason"
    );
}

#[test]
fn quiet_cause_class_changes_remain_visible_but_payload_counts_do_not_replay() {
    let mut printed = Printed::default();
    let first = [unknown("idle", Cause::JournalPartial { skipped: 1 })];
    assert!(show(&mut printed, &first, 0).contains("journal partial"));
    let same_class = [unknown("idle", Cause::JournalPartial { skipped: 8 })];
    assert!(
        show(&mut printed, &same_class, 1).is_empty(),
        "cause class, not fluctuating skipped count, is news"
    );
    let changed = [unknown("idle", Cause::RuntimeUnread)];
    assert!(
        show(&mut printed, &changed, 2).contains("tmux"),
        "new uncertainty class stays explicit"
    );
}

#[test]
fn quiet_clearing_real_work_is_visible_even_while_uncertainty_stands() {
    let mut printed = Printed::default();
    let gap = unknown("idle", Cause::WatchdogOff);
    let rows = [reason("lead", Reason::WaitingUser), gap.clone()];
    let _ = show(&mut printed, &rows, 0);
    let text = show(&mut printed, &[gap], 1);
    assert!(
        text.contains("nothing standing") || text.contains("-- needs you: 0 seats"),
        "cleared human work must not leave stale action count: {text}"
    );
    assert!(!text.contains("ACTION-lead"));
}

#[test]
fn quiet_resume_gives_watchdog_awake_time_then_reports_real_silence() {
    let mut printed = Printed::default();
    let rows = [reason("lead", Reason::WaitingUser)];
    let _ = show(&mut printed, &rows, 0);
    let asleep = 3_600;
    let stale = stale_rows(&rows, 0);
    assert!(
        show(&mut printed, &stale, asleep).is_empty(),
        "first tick after host sleep is not watchdog failure"
    );
    let mut observed = String::new();
    for tick in 1..=STALE_SECS + 5 {
        let text = show(&mut printed, &stale, asleep + tick);
        if tick <= 5 {
            assert!(text.is_empty(), "watchdog gets time to wake before blame");
        }
        observed.push_str(&text);
    }
    single_silence_notice(&observed);
}

#[test]
fn quiet_resume_does_not_hide_watchdog_off_or_unreadable_evidence() {
    for cause in [Cause::WatchdogOff, Cause::WatchdogUnreadable] {
        let mut printed = Printed::default();
        let _ = show(&mut printed, &[], 0);
        let text = show(&mut printed, &[unknown("idle", cause)], 3_600);
        assert!(
            text.contains("watchdog") && text.contains("idle"),
            "resume grace is only for silence, not missing or unreadable evidence: {text}"
        );
    }
}

#[test]
fn quiet_resume_grace_reports_changed_real_verdict_on_the_first_awake_tick() {
    let mut printed = Printed::default();
    let before = [reason("lead", Reason::WaitingUser)];
    let _ = show(&mut printed, &before, 0);
    let after = [reason("lead", Reason::Dead)];
    let text = show(&mut printed, &stale_rows(&after, 0), 3_600);
    assert!(
        text.contains("-- needs you: 1 seat")
            && text.contains("dead")
            && text.contains("ACTION-lead"),
        "sleep explains silence, never delays new real human work: {text}"
    );
}

#[test]
fn quiet_new_unverified_seat_under_same_cause_class_does_not_replay() {
    let mut printed = Printed::default();
    let _ = show(&mut printed, &[unknown("one", Cause::WatchdogOff)], 0);
    let same_doubt = [
        unknown("one", Cause::WatchdogOff),
        unknown("two", Cause::WatchdogOff),
    ];
    assert!(
        show(&mut printed, &same_doubt, 1).is_empty(),
        "already-disclosed uncertainty class alone is not new human work"
    );
}

#[test]
fn quiet_exactly_sixty_seconds_between_ticks_does_not_start_resume_grace() {
    let mut printed = Printed::default();
    let rows = [reason("lead", Reason::WaitingUser)];
    let _ = show(&mut printed, &rows, 0);
    let stale = stale_rows(&rows, -3_600);
    single_silence_notice(&show(&mut printed, &stale, 60));
    assert!(
        show(&mut printed, &stale, 61).is_empty(),
        "silence already disclosed once at the strict resume boundary"
    );
}

#[test]
fn quiet_resume_grace_ends_at_exactly_one_hundred_eighty_awake_seconds() {
    let mut printed = Printed::default();
    let rows = [reason("lead", Reason::WaitingUser)];
    let _ = show(&mut printed, &rows, 0);
    let stale = stale_rows(&rows, -3_600);
    let resumed = 3_600;
    assert!(show(&mut printed, &stale, resumed).is_empty());
    for awake in (5..180).step_by(5) {
        assert!(
            show(&mut printed, &stale, resumed + awake).is_empty(),
            "resume grace holds before its declared duration"
        );
    }
    assert!(
        show(&mut printed, &stale, resumed + 179).is_empty(),
        "watchdog still has awake time at 179 seconds"
    );
    single_silence_notice(&show(&mut printed, &stale, resumed + 180));
    assert!(
        show(&mut printed, &stale, resumed + 181).is_empty(),
        "grace expiry discloses silence once"
    );
}

struct TranscriptRig {
    _owned: super::cli::OwnedScratch,
    root: PathBuf,
    transcript: PathBuf,
    sessions: Vec<ae::usage::SessionInput>,
}

impl TranscriptRig {
    fn new(tag: &str, body: &str) -> Self {
        let owned = super::cli::OwnedScratch::root("chatquiet", tag);
        let root = owned.path().to_owned();
        let store = root.join("claude-store");
        let id = "0199c0de-aaaa-4890-abcd-ef0123456789";
        let roster = super::board::claude_roster("main", "lead", id, &store);
        let session = super::board::plant_session(&root, "quiet", &roster);
        let transcript = super::board::plant_transcript(&store, "synthetic", id, &[]);
        replace_bytes(&transcript, body, 10);
        Self {
            _owned: owned,
            root,
            transcript,
            sessions: vec![ae::usage::SessionInput {
                name: "quiet".to_owned(),
                path: session,
            }],
        }
    }

    fn inputs(&self) -> ae::board::Inputs<'_> {
        ae::board::Inputs {
            home: Some(&self.root),
            sessions: &self.sessions,
            replies: ae::board::Replies::All,
        }
    }

    fn grok(tag: &str, body: &str) -> Self {
        let owned = super::cli::OwnedScratch::root("chatquiet", tag);
        let root = owned.path().to_owned();
        let id = "0199c0de-aaaa-4890-abcd-ef0123456789";
        let dir = root.join(".grok/sessions/work").join(id);
        std::fs::create_dir_all(&dir).expect("own synthetic grok store");
        let transcript = dir.join("updates.jsonl");
        replace_bytes(&transcript, body, 10);
        let roster = format!("seat.main=lead\nagent_bin.main=grok\nharness_session.main={id}\n");
        let session = super::board::plant_session(&root, "quiet", &roster);
        Self {
            _owned: owned,
            root,
            transcript,
            sessions: vec![ae::usage::SessionInput {
                name: "quiet".to_owned(),
                path: session,
            }],
        }
    }

    fn seed(&self) -> ae::board::follow::Follow {
        let first = ae::board::observe(&self.inputs(), None);
        assert!(
            first.coverage.is_empty(),
            "synthetic first read is complete: {:?}",
            first.coverage
        );
        assert_eq!(first.rows.len(), 1, "synthetic human row found");
        ae::board::follow::Follow::seeded(&first.seeds, &first.coverage, None)
    }

    fn poll(&self, follow: &mut ae::board::follow::Follow) -> ae::board::Observation {
        ae::board::follow_poll(&self.inputs(), follow)
    }
}

// Same path and inode; explicit mtimes make this independent of filesystem
// timestamp resolution. Identical bytes represent a touch-only final state.
fn replace_bytes(path: &Path, body: &str, tick: u64) {
    let mut file = std::fs::File::create(path).expect("own synthetic transcript");
    file.write_all(body.as_bytes())
        .expect("synthetic transcript bytes");
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(tick))
        .expect("controlled fixture mtime");
}

fn human(body: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"timestamp\":\"2026-10-03T18:00:00Z\",\"message\":{{\"role\":\"user\",\"content\":\"{body}\"}}}}\n"
    )
}

#[test]
fn quiet_identical_bytes_with_changed_mtime_emit_no_rows_or_gap() {
    let bytes = human("ORIGINAL");
    let rig = TranscriptRig::new("touch", &bytes);
    let mut follow = rig.seed();
    for tick in [20, 30, 40] {
        replace_bytes(&rig.transcript, &bytes, tick);
        let read = rig.poll(&mut follow);
        assert!(
            read.coverage.is_empty(),
            "mtime-only change is not incomplete coverage: {:?}",
            read.coverage
        );
        assert!(
            read.rows.is_empty(),
            "identical bytes must not replay transcript rows"
        );
    }
}

#[test]
fn quiet_identical_empty_transcript_touch_emits_no_rows_or_gap() {
    let rig = TranscriptRig::new("emptytouch", "");
    let initial = ae::board::observe(&rig.inputs(), None);
    assert!(initial.rows.is_empty());
    assert_eq!(
        initial.seeds.len(),
        1,
        "empty transcript successfully observed"
    );
    let mut follow = ae::board::follow::Follow::seeded(&initial.seeds, &initial.coverage, None);
    for tick in [20, 30] {
        replace_bytes(&rig.transcript, "", tick);
        let touched = rig.poll(&mut follow);
        assert!(
            touched.rows.is_empty() && touched.coverage.is_empty(),
            "identical empty bytes are a touch, not a generation gap: {:?}",
            touched.coverage
        );
    }
}

#[test]
fn quiet_same_length_rewrite_reports_once_and_shows_replacement_bytes() {
    let before = human("ORIGINAL");
    let after = human("REPLACED");
    assert_eq!(
        before.len(),
        after.len(),
        "contract targets same-length rewrite"
    );
    let rig = TranscriptRig::new("rewrite", &before);
    let mut follow = rig.seed();
    replace_bytes(&rig.transcript, &after, 20);
    let changed = rig.poll(&mut follow);
    assert_eq!(
        changed.coverage.len(),
        1,
        "real rewrite reports once: {:?}",
        changed.coverage
    );
    assert!(changed.coverage[0].reason.contains("transcript rewritten"));
    assert_eq!(
        changed
            .rows
            .iter()
            .map(|row| row.body.as_str())
            .collect::<Vec<_>>(),
        ["REPLACED"]
    );
    replace_bytes(&rig.transcript, &after, 30);
    let unchanged = rig.poll(&mut follow);
    assert!(
        unchanged.rows.is_empty() && unchanged.coverage.is_empty(),
        "touch after rewrite is quiet"
    );
}

#[test]
fn quiet_touch_then_append_preserves_offset_and_only_shows_new_rows() {
    let first = human("FIRST");
    let rig = TranscriptRig::new("append", &first);
    let mut follow = rig.seed();
    replace_bytes(&rig.transcript, &first, 20);
    let _ = rig.poll(&mut follow);
    let appended = format!("{first}{}", human("SECOND"));
    replace_bytes(&rig.transcript, &appended, 30);
    let read = rig.poll(&mut follow);
    assert!(read.coverage.is_empty(), "append is complete coverage");
    assert_eq!(
        read.rows
            .iter()
            .map(|row| row.body.as_str())
            .collect::<Vec<_>>(),
        ["SECOND"]
    );
}

#[test]
fn quiet_rewrite_of_malformed_committed_bytes_still_reports_generation_change() {
    let before = format!("{}malformed A\n", human("FIRST"));
    let after = format!("{}malformed B\n", human("FIRST"));
    let rig = TranscriptRig::new("damaged", &before);
    let first = ae::board::observe(&rig.inputs(), None);
    assert_eq!(
        first
            .rows
            .iter()
            .map(|row| row.body.as_str())
            .collect::<Vec<_>>(),
        ["FIRST"],
        "malformed tail leaves retained human rows unchanged"
    );
    let mut follow = ae::board::follow::Follow::seeded(&first.seeds, &first.coverage, None);
    replace_bytes(&rig.transcript, &after, 20);
    let changed = rig.poll(&mut follow);
    assert!(
        changed
            .coverage
            .iter()
            .any(|gap| gap.reason.contains("transcript rewritten")),
        "raw committed bytes changed even though retained rows are identical: {:?}",
        changed.coverage
    );
}

#[test]
fn quiet_single_low_bit_change_in_raw_committed_bytes_reports_rewrite_once() {
    let before = format!("{}malformed A\n", human("FIRST"));
    let after = format!("{}malformed @\n", human("FIRST"));
    assert_eq!(before.len(), after.len());
    let rig = TranscriptRig::new("rawlowbit", &before);
    let initial = ae::board::observe(&rig.inputs(), None);
    assert_eq!(
        initial
            .rows
            .iter()
            .map(|row| row.body.as_str())
            .collect::<Vec<_>>(),
        ["FIRST"]
    );
    let mut follow = ae::board::follow::Follow::seeded(&initial.seeds, &initial.coverage, None);
    replace_bytes(&rig.transcript, &after, 20);
    let changed = rig.poll(&mut follow);
    assert!(
        changed
            .coverage
            .iter()
            .any(|gap| gap.reason.contains("transcript rewritten")),
        "one changed committed byte reports a real rewrite: {:?}",
        changed.coverage
    );
    assert_eq!(
        changed
            .rows
            .iter()
            .map(|row| row.body.as_str())
            .collect::<Vec<_>>(),
        ["FIRST"],
        "retained rows stay identical while raw evidence changes"
    );
    replace_bytes(&rig.transcript, &after, 30);
    let touched = rig.poll(&mut follow);
    assert!(
        touched.rows.is_empty() && touched.coverage.is_empty(),
        "the generation change is reported once"
    );
}

#[test]
fn quiet_rewrite_in_an_earlier_stream_chunk_is_not_misreported_as_a_touch() {
    // Several read chunks belong to one committed, unrecognized line. Only
    // its early bytes change; retained rows and the final chunk stay identical.
    let tail = "x".repeat(131_072);
    let before = format!("{}malformed A{tail}\n", human("FIRST"));
    let after = format!("{}malformed B{tail}\n", human("FIRST"));
    assert_eq!(before.len(), after.len());
    let rig = TranscriptRig::new("earlychunk", &before);
    let mut follow = rig.seed();
    replace_bytes(&rig.transcript, &after, 20);
    let changed = rig.poll(&mut follow);
    assert!(
        changed
            .coverage
            .iter()
            .any(|gap| gap.reason.contains("transcript rewritten")),
        "every committed raw byte affects identity, including earlier chunks: {:?}",
        changed.coverage
    );
    assert_eq!(
        changed
            .rows
            .iter()
            .map(|row| row.body.as_str())
            .collect::<Vec<_>>(),
        ["FIRST"],
        "a real rewrite rescans even when retained rows are unchanged"
    );
}

#[test]
fn quiet_torn_tail_across_chunks_keeps_the_commit_until_completion() {
    let first = human("FIRST");
    let body = format!("SECOND{}", "x".repeat(131_072));
    let second = human(&body);
    let torn = format!("{first}{}", second.trim_end_matches('\n'));
    let rig = TranscriptRig::new("tornchunks", &torn);
    let initial = ae::board::observe(&rig.inputs(), None);
    assert_eq!(
        initial
            .rows
            .iter()
            .map(|row| row.body.as_str())
            .collect::<Vec<_>>(),
        ["FIRST"]
    );
    assert!(
        initial
            .coverage
            .iter()
            .any(|gap| gap.reason.contains("torn last record")),
        "only complete records are trusted"
    );
    let mut follow = ae::board::follow::Follow::seeded(&initial.seeds, &initial.coverage, None);
    replace_bytes(&rig.transcript, &torn, 20);
    let touched = rig.poll(&mut follow);
    assert!(
        touched.rows.is_empty() && touched.coverage.is_empty(),
        "an identical torn tail neither replays rows nor adds a gap"
    );
    let completed = format!("{first}{second}");
    replace_bytes(&rig.transcript, &completed, 30);
    let done = rig.poll(&mut follow);
    assert!(done.coverage.is_empty(), "completion is an append");
    assert_eq!(done.rows.len(), 1, "completed record handled once");
    assert_eq!(done.rows[0].body, body);
    assert_eq!(
        done.rows[0].offset,
        first.len() as u64,
        "commit offset kept"
    );
    replace_bytes(&rig.transcript, &completed, 40);
    let touched_after_completion = rig.poll(&mut follow);
    assert!(
        touched_after_completion.rows.is_empty() && touched_after_completion.coverage.is_empty(),
        "completed identity must exclude the formerly uncommitted tail: {:?}",
        touched_after_completion.coverage
    );
}

#[test]
fn quiet_rewrite_beyond_overlong_line_cap_is_not_misreported_as_a_touch() {
    // Deliberately beyond the documented 1 MiB reader cap; only the final byte
    // changes, so comparing retained prefixes or overlong lengths cannot pass.
    let long = "x".repeat(1_048_576 + 32);
    let before = format!("{}{long}A\n", human("FIRST"));
    let after = format!("{}{long}B\n", human("FIRST"));
    assert_eq!(before.len(), after.len());
    let rig = TranscriptRig::new("overlong", &before);
    let first = ae::board::observe(&rig.inputs(), None);
    assert!(
        !first.coverage.is_empty(),
        "overlong fixture names omitted bytes"
    );
    let mut follow = ae::board::follow::Follow::seeded(&first.seeds, &first.coverage, None);
    replace_bytes(&rig.transcript, &after, 20);
    let changed = rig.poll(&mut follow);
    assert!(
        changed
            .coverage
            .iter()
            .any(|gap| gap.reason.contains("transcript rewritten")),
        "discarded parser bytes still belong to committed transcript identity: {:?}",
        changed.coverage
    );
}

fn grok_line(kind: &str, text: Option<&str>) -> String {
    let content = text.map_or_else(String::new, |text| {
        format!(r#","content":{{"type":"text","text":"{text}"}}"#)
    });
    format!(
        r#"{{"timestamp":1789549200,"method":"session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"{kind}"{content}}}}}}}"#
    ) + "\n"
}

#[test]
fn quiet_grok_fragment_rewind_completes_then_identical_touch_stays_quiet() {
    let first = grok_line("user_message_chunk", Some("typed"));
    let open = format!("{first}{}", grok_line("agent_message_chunk", Some("frag")));
    let completed = format!(
        "{open}{}{}",
        grok_line("agent_message_chunk", Some("ment")),
        grok_line("turn_completed", None)
    );
    for seed_open in [true, false] {
        let rig = TranscriptRig::grok(
            if seed_open { "grokseed" } else { "grokpoll" },
            if seed_open { &open } else { &first },
        );
        let mut follow = if seed_open {
            let initial = ae::board::observe(&rig.inputs(), None);
            assert!(initial.coverage.is_empty());
            assert_eq!(
                initial
                    .rows
                    .iter()
                    .map(|row| (row.body.as_str(), row.offset))
                    .collect::<Vec<_>>(),
                [("typed", 0), ("frag", first.len() as u64)],
                "the one-shot shows the open run before follow holds it"
            );
            ae::board::follow::Follow::seeded(&initial.seeds, &initial.coverage, None)
        } else {
            rig.seed()
        };
        if !seed_open {
            replace_bytes(&rig.transcript, &open, 20);
            let fragment = rig.poll(&mut follow);
            assert!(
                fragment.rows.is_empty(),
                "open assistant fragment stays held"
            );
            assert!(fragment.coverage.is_empty());
        }
        replace_bytes(&rig.transcript, &completed, 30);
        let done = rig.poll(&mut follow);
        assert!(done.coverage.is_empty());
        assert_eq!(
            done.rows
                .iter()
                .map(|row| row.body.as_str())
                .collect::<Vec<_>>(),
            ["fragment"],
            "fragment joins once after completion"
        );
        replace_bytes(&rig.transcript, &completed, 40);
        let touched = rig.poll(&mut follow);
        assert!(
            touched.rows.is_empty() && touched.coverage.is_empty(),
            "normal rewind must retain committed-prefix identity: {:?}",
            touched.coverage
        );
    }
}
