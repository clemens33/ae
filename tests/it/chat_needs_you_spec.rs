//! Independent acceptance for the full-roster attention snapshot and `/open`.
//! Oracle: N1–N12 and lead Q1/Q4/Q5. The pure seams pin reusable data; real
//! private tmux fixtures below pin chat wiring and session-scoped selection.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures own private files, sockets and synthetic records"
)]

use std::fmt::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ae::attention::Reason;
use ae::console::input::{Command, Effect, Input, Reading, Size};
use ae::console::needs::{self, Cause, Inputs, Row, SeatRef, Section, Source, Verdict};
use ae::console::open::{self, Facts, Refusal, Target};
use ae::console::view::{Printed, Style};
use ae::digest::{SessionEntry, Status};
use ae::events::{Cursor, Event};
use ae::inventory::{MetaRead, ServerId};
use ae::meta::{Meta, Selector};
use ae::session::{AgentRuntime, RecordSnapshot, SessionRead, SessionRuntime};
use ae::time::Timestamp;
use ae::tmux::{Evidence, ObservedSlot, PickerPane, WindowPane};

const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const OTHER_UUID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const LIMIT: Duration = Duration::from_secs(8);

fn stamp(text: &str) -> Timestamp {
    Timestamp::parse(text).expect("frozen UTC timestamp")
}

fn now() -> Timestamp {
    stamp("2026-10-03T18:00:00Z")
}

fn escaped(text: &str) -> String {
    let mut out = String::new();
    ae::json::escape_into(text, &mut out);
    out
}

fn event(ts: &str, actor: &str, action: &str, target: &str, state: &str, body: &str) -> Event {
    let line = format!(
        "{{\"ts\":\"{ts}\",\"actor\":\"{}\",\"action\":\"{}\",\"target\":\"{}\",\"ref\":\"{}\",\"summary\":\"{}\"}}",
        escaped(actor),
        escaped(action),
        escaped(target),
        escaped(state),
        escaped(body),
    );
    Event::parse_line(&line).expect("synthetic record parses")
}

struct FoldRig {
    snapshot: RecordSnapshot,
    runtime: SessionRuntime,
    entry: SessionEntry,
}

impl FoldRig {
    fn new(seats: &[(&str, &str)], events: Vec<Event>) -> Self {
        let mut raw = format!("schema=2\nsession_id={UUID}\nlayout=lead-pair\n");
        for (slot, name) in seats {
            writeln!(raw, "seat.{slot}={name}\nprofile.{slot}=idle").expect("String write");
        }
        let snapshot = RecordSnapshot {
            meta: Some(Meta::parse(&raw)),
            meta_read: MetaRead::Parsed,
            events: Some(SessionRead {
                last_active: events.iter().map(|event| event.ts).max(),
                events,
                pending: Vec::new(),
                cursor: Cursor::default(),
                skipped: Vec::new(),
            }),
            legacy_created_epoch: None,
        };
        let runtime = SessionRuntime {
            status: Status::Running,
            branch: Some("fixture".to_owned()),
            agents: seats
                .iter()
                .map(|(slot, _)| AgentRuntime {
                    slot: (*slot).to_owned(),
                    alive: Some(true),
                    alert: None,
                    observed: ae::harness_state::HarnessState::Unknown,
                })
                .collect(),
        };
        let entry = ae::session::entry_from(&snapshot, "one", &runtime, now(), 1800);
        Self {
            snapshot,
            runtime,
            entry,
        }
    }

    fn read(&self, beat: Evidence, runtime_read: bool) -> Result<Section, String> {
        needs::fold(&Inputs {
            session: "one",
            snapshot: &self.snapshot,
            entry: &self.entry,
            runtime: &self.runtime,
            runtime_read,
            beat,
            lead_pair: true,
            now: now(),
        })
    }

    fn settled(&self) -> Section {
        self.read(Evidence::At(now().epoch()), true)
            .expect("settled snapshot")
    }
}

#[test]
fn needs_fold_uses_supplied_reason_for_every_roster_seat_including_spawned() {
    let mut rig = FoldRig::new(
        &[
            ("main", "lead"),
            ("worker.0", "colead"),
            ("spawned.0", "scout"),
        ],
        Vec::new(),
    );
    for reason in Reason::BY_SEVERITY {
        for agent in &mut rig.entry.agents {
            agent.reason = Some(reason);
        }
        let section = rig.settled();
        assert_eq!(
            section.rows.len(),
            3,
            "each existing reason needs the human: {reason}"
        );
        assert!(
            section
                .rows
                .iter()
                .all(|row| row.verdict == Verdict::Reason(reason))
        );
        assert_eq!(
            section
                .rows
                .iter()
                .map(|row| row.seat.name.as_str())
                .collect::<Vec<_>>(),
            ["lead", "colead", "scout"]
        );
        assert!(section.rows[0].lead_pair && section.rows[1].lead_pair);
        assert!(
            !section.rows[2].lead_pair,
            "a spawned row never becomes conversation pair"
        );
    }
}

#[test]
fn needs_fold_does_not_reclassify_events_against_the_supplied_owner_verdict() {
    let events = vec![event(
        "2026-10-03T17:55:00Z",
        "scout",
        "state",
        "",
        "waiting-user",
        "SHOULD-NOT-DECIDE",
    )];
    let mut rig = FoldRig::new(&[("spawned.0", "scout")], events);
    rig.entry.agents[0].reason = None;
    assert!(
        rig.settled().rows.is_empty(),
        "the fold must not grow a second classifier"
    );
    rig.entry.agents[0].reason = Some(Reason::Dead);
    let section = rig.settled();
    assert_eq!(section.rows.len(), 1);
    assert_eq!(section.rows[0].verdict, Verdict::Reason(Reason::Dead));
}

#[test]
fn needs_fold_sorts_numeric_rank_then_oldest_age_not_reason_ord_or_name() {
    let events = vec![
        event(
            "2026-10-03T17:55:00Z",
            "a-blocked",
            "state",
            "",
            "blocked",
            "new block",
        ),
        event(
            "2026-10-03T17:05:00Z",
            "watchdog",
            "limit",
            "z-limit",
            "",
            "old limit",
        ),
        event(
            "2026-10-03T17:59:00Z",
            "watchdog",
            "alert",
            "dead-last",
            "",
            "dead pane vanished",
        ),
        event(
            "2026-10-03T17:10:00Z",
            "watchdog",
            "limit",
            "b-limit",
            "",
            "newer limit",
        ),
    ];
    let rig = FoldRig::new(
        &[
            ("main", "a-blocked"),
            ("spawned.0", "z-limit"),
            ("spawned.1", "dead-last"),
            ("spawned.2", "b-limit"),
        ],
        events,
    );
    let section = rig.settled();
    assert_eq!(
        section
            .rows
            .iter()
            .map(|row| row.seat.name.as_str())
            .collect::<Vec<_>>(),
        ["dead-last", "z-limit", "b-limit", "a-blocked"]
    );
    assert_eq!(
        section.rows[1].since_micros,
        Some(stamp("2026-10-03T17:05:00Z").epoch() * 1_000_000)
    );
}

#[test]
fn needs_fold_attributes_human_prompt_and_current_declaration_with_source_age_and_body() {
    let events = vec![
        event(
            "2026-10-03T17:40:00Z",
            "watchdog",
            "human-prompt",
            "scout",
            "",
            "TRUST-MODAL",
        ),
        event(
            "2026-10-03T17:50:00Z",
            "lead",
            "state",
            "",
            "waiting-user",
            "FULL-QUESTION",
        ),
    ];
    let rig = FoldRig::new(&[("main", "lead"), ("spawned.0", "scout")], events);
    let rows = rig.settled().rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].source, Source::Declaration);
    assert_eq!(rows[0].detail, "FULL-QUESTION");
    assert_eq!(rows[0].record, Some(1));
    assert_eq!(rows[1].verdict, Verdict::Reason(Reason::Blocked));
    assert_eq!(
        rows[1].source,
        Source::Alert {
            action: "human-prompt".to_owned()
        }
    );
    assert_eq!(
        rows[1].since_micros,
        Some(stamp("2026-10-03T17:40:00Z").epoch() * 1_000_000)
    );
    assert_eq!(rows[1].detail, "TRUST-MODAL");
    assert_eq!(rows[1].record, Some(0));
}

#[test]
fn needs_fold_cleared_alert_and_superseded_declaration_leave_the_snapshot() {
    let events = vec![
        event(
            "2026-10-03T17:40:00Z",
            "watchdog",
            "human-prompt",
            "scout",
            "",
            "old modal",
        ),
        event(
            "2026-10-03T17:41:00Z",
            "watchdog",
            "human-prompt-cleared",
            "scout",
            "",
            "cleared",
        ),
        event(
            "2026-10-03T17:50:00Z",
            "lead",
            "state",
            "",
            "waiting-user",
            "old question",
        ),
        event(
            "2026-10-03T17:51:00Z",
            "lead",
            "state",
            "",
            "working",
            "answered",
        ),
    ];
    let rig = FoldRig::new(&[("main", "lead"), ("spawned.0", "scout")], events);
    assert!(
        rig.entry.agents.iter().all(|agent| agent.reason.is_none()),
        "existing classifier fixture is settled"
    );
    assert!(rig.settled().rows.is_empty());
}

#[test]
fn needs_fold_unknown_sources_are_individual_rows_before_view_collapse() {
    let rig = FoldRig::new(
        &[
            ("main", "lead"),
            ("worker.0", "colead"),
            ("spawned.0", "scout"),
        ],
        Vec::new(),
    );
    for (beat, cause) in [
        (Evidence::Silent, Cause::WatchdogOff),
        (Evidence::Unreadable, Cause::WatchdogUnreadable),
        (
            Evidence::At(now().epoch() - 181),
            Cause::WatchdogStale {
                last_micros: (now().epoch() - 181) * 1_000_000,
            },
        ),
    ] {
        let section = rig.read(beat, true).expect("readable journal");
        assert_eq!(
            section.rows.len(),
            3,
            "all unverified seats stay in reusable data: {cause:?}"
        );
        for row in section.rows {
            assert_eq!(row.verdict, Verdict::Unknown(cause));
            assert_eq!(row.source, Source::Unverified);
        }
    }
    assert!(
        rig.settled().rows.is_empty(),
        "fresh verified no-reason seats need no row"
    );
    assert!(
        rig.read(Evidence::At(now().epoch() - 180), true)
            .expect("readable journal")
            .rows
            .is_empty(),
        "exactly three verdict intervals remains fresh"
    );
}

#[test]
fn needs_fold_runtime_unknown_and_known_reason_with_stale_source_never_look_fine() {
    let mut rig = FoldRig::new(&[("main", "lead"), ("spawned.0", "scout")], Vec::new());
    let rows = rig
        .read(Evidence::At(now().epoch()), false)
        .expect("readable journal")
        .rows;
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.verdict == Verdict::Unknown(Cause::RuntimeUnread))
    );
    rig.entry.agents[1].reason = Some(Reason::Limit);
    let rows = rig
        .read(Evidence::Silent, true)
        .expect("readable journal")
        .rows;
    let scout = rows
        .iter()
        .find(|row| row.seat.name == "scout")
        .expect("known seat retained");
    assert_eq!(scout.verdict, Verdict::Reason(Reason::Limit));
    assert_eq!(scout.stale, Some(Cause::WatchdogOff));
    rig.snapshot.events = None;
    let read = rig.read(Evidence::At(now().epoch()), true);
    assert!(
        read.is_err(),
        "unreadable journal cannot prove an empty section"
    );
}

#[test]
fn needs_fold_missing_roster_and_partial_or_unproven_facts_cannot_clear_silently() {
    let mut rig = FoldRig::new(&[("main", "lead"), ("spawned.0", "scout")], Vec::new());
    rig.runtime.agents[1].alive = None;
    let rows = rig.settled().rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].seat.name, "scout");
    assert_eq!(rows[0].verdict, Verdict::Unknown(Cause::PaneUnproven));
    rig.runtime.agents[1].alive = Some(true);
    rig.snapshot
        .events
        .as_mut()
        .expect("journal fixture")
        .skipped
        .push(ae::events::SkippedLine {
            generation: 0,
            offset: 12,
            reason: ae::events::EventError::MissingKey("ts"),
        });
    let rows = rig.settled().rows;
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.verdict == Verdict::Unknown(Cause::JournalPartial { skipped: 1 }))
    );
    rig.snapshot.meta = None;
    rig.snapshot.meta_read = MetaRead::Unreadable;
    assert!(
        rig.read(Evidence::At(now().epoch()), true).is_err(),
        "absent roster is an unsettled read, never an empty snapshot"
    );
}

fn row(name: &str, slot: &str, reason: Reason, detail: &str) -> Row {
    Row {
        seat: SeatRef {
            slot: slot.to_owned(),
            name: name.to_owned(),
        },
        lead_pair: slot == "main" || slot == "worker.0",
        verdict: Verdict::Reason(reason),
        source: Source::Declaration,
        stale: None,
        since_micros: Some(stamp("2026-10-03T17:50:00Z").epoch() * 1_000_000),
        detail: detail.to_owned(),
        record: Some(0),
    }
}

fn strip_sgr(text: &str) -> String {
    let mut chars = text.chars();
    let mut plain = String::new();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            plain.push(ch);
            continue;
        }
        assert_eq!(
            chars.next(),
            Some('['),
            "only Style's SGR escapes may survive"
        );
        let mut ended = false;
        for part in chars.by_ref() {
            if part == 'm' {
                ended = true;
                break;
            }
            assert!(
                part.is_ascii_digit() || part == ';',
                "record escape reached terminal"
            );
        }
        assert!(ended, "SGR terminates");
    }
    plain
}

#[test]
fn needs_view_collapses_one_cause_but_snapshot_still_contains_all_names() {
    let rows = ["lead", "colead", "scout"]
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let mut row = row(name, &format!("spawned.{index}"), Reason::Blocked, "");
            row.verdict = Verdict::Unknown(Cause::WatchdogOff);
            row.source = Source::Unverified;
            row
        })
        .collect();
    let mut printed = Printed::default();
    let text = printed.needs(
        &Ok(Section { rows }),
        Some(Size {
            width: 120,
            height: 30,
        }),
        now(),
    );
    assert!(
        text.contains("3 seats") && text.contains("unverified"),
        "collapsed cause still counts all seats: {text}"
    );
    let cause_lines: Vec<_> = text
        .lines()
        .filter(|line| line.contains("watchdog") && line.contains("3 seats"))
        .collect();
    assert_eq!(
        cause_lines.len(),
        1,
        "one cause produces one gap line: {text}"
    );
    assert!(
        cause_lines[0].contains("lead")
            && cause_lines[0].contains("colead")
            && cause_lines[0].contains("scout")
    );
}

#[test]
fn needs_view_unsettled_read_warns_at_once_and_only_settled_read_closes_rows() {
    let mut printed = Printed::default();
    let failed = Err("journal unreadable".to_owned());
    let cold = printed.needs(&failed, None, now());
    assert!(
        cold.contains("journal unreadable"),
        "first unreadable read must name its gap"
    );
    assert!(
        printed.needs(&failed, None, now()).is_empty(),
        "same gap once per episode"
    );
    let standing = Ok(Section {
        rows: vec![row("scout", "spawned.0", Reason::Blocked, "RETAIN-ROW")],
    });
    assert!(printed.needs(&standing, None, now()).contains("RETAIN-ROW"));
    let gap = printed.needs(&failed, None, now());
    assert!(gap.contains("journal unreadable") && !gap.contains("nothing standing"));
    assert!(
        printed.needs(&standing, None, now()).is_empty(),
        "unsettled read retained row identity"
    );
    let cleared = printed.needs(&Ok(Section::default()), None, now());
    assert!(cleared.contains("nothing standing"));
    assert!(
        printed
            .needs(&Ok(Section::default()), None, now())
            .is_empty()
    );
}

#[test]
fn needs_view_distinct_causes_remain_named_and_elapsed_age_alone_does_not_repaint() {
    let mut off = row("scout", "spawned.0", Reason::Blocked, "");
    off.verdict = Verdict::Unknown(Cause::WatchdogOff);
    off.source = Source::Unverified;
    let mut unproven = row("builder", "spawned.1", Reason::Blocked, "");
    unproven.verdict = Verdict::Unknown(Cause::PaneUnproven);
    unproven.source = Source::Unverified;
    let section = Ok(Section {
        rows: vec![off, unproven],
    });
    let mut printed = Printed::default();
    let text = printed.needs(
        &section,
        Some(Size {
            width: 120,
            height: 30,
        }),
        now(),
    );
    assert!(
        text.lines()
            .any(|line| line.contains("scout") && line.contains("watchdog"))
    );
    assert!(
        text.lines()
            .any(|line| line.contains("builder") && line.contains("pane"))
    );
    assert!(
        printed
            .needs(
                &section,
                Some(Size {
                    width: 120,
                    height: 30
                }),
                stamp("2026-10-03T18:01:00Z")
            )
            .is_empty()
    );
}

#[test]
fn needs_view_repaints_changed_detail_source_and_freshness_with_same_record() {
    let mut printed = Printed::default();
    let mut section = Section {
        rows: vec![row("scout", "spawned.0", Reason::Blocked, "OLD-DETAIL")],
    };
    assert!(
        printed
            .needs(&Ok(section.clone()), None, now())
            .contains("OLD-DETAIL")
    );
    section.rows[0].detail = "NEW-DETAIL".to_owned();
    assert!(
        printed
            .needs(&Ok(section.clone()), None, now())
            .contains("NEW-DETAIL")
    );
    section.rows[0].source = Source::Alert {
        action: "human-prompt".to_owned(),
    };
    assert!(
        printed
            .needs(&Ok(section.clone()), None, now())
            .contains("human prompt")
    );
    section.rows[0].stale = Some(Cause::WatchdogOff);
    let changed = printed.needs(&Ok(section), None, now());
    assert!(
        changed.contains("watchdog") && changed.contains("stale"),
        "freshness is printed evidence: {changed}"
    );
}

#[test]
fn needs_view_dresses_terminal_and_neutralises_every_record_field_in_plain_output() {
    let mut hostile = row(
        "scout\x1b]52;c;AAAA\x07",
        "spawned.0",
        Reason::Blocked,
        "before\x1b[2Jafter\t中\nnext",
    );
    hostile.source = Source::Alert {
        action: "human-prompt\x1b[31m".to_owned(),
    };
    let section = Ok(Section {
        rows: vec![hostile],
    });
    let plain = Printed::default().needs(&section, None, now());
    assert!(!plain.contains('\x1b') && !plain.contains('\x07') && !plain.contains('\t'));
    assert!(
        plain.contains("scout") && plain.contains("before"),
        "neutralisation does not drop the row"
    );
    let style = Style::resolve(true, None, Some("+0000"), "lead");
    let drawn = Printed::styled(style).needs(&section, None, now());
    assert!(drawn.contains("\x1b["), "terminal uses the one Style");
    let inert = strip_sgr(&drawn);
    assert!(inert.contains("before") && !inert.contains('\x07') && !inert.contains('\t'));
}

#[test]
fn needs_view_long_roster_reports_omissions_and_tiny_pane_keeps_count() {
    let section = Ok(Section {
        rows: (0..20)
            .map(|index| {
                row(
                    &format!("z{index:02}"),
                    &format!("spawned.{index}"),
                    Reason::Blocked,
                    "detail",
                )
            })
            .collect(),
    });
    let text = Printed::default().needs(
        &section,
        Some(Size {
            width: 120,
            height: 24,
        }),
        now(),
    );
    assert!(text.contains("needs you"), "section rendered");
    assert!(text.lines().count() <= 8, "height/3 physical rows: {text}");
    let shown = (0..20)
        .filter(|index| text.contains(&format!("z{index:02}")))
        .count();
    assert!(shown > 0 && shown < 20);
    assert!(
        text.contains(&format!("{} more", 20 - shown)),
        "omitted count matches hidden seats: {text}"
    );
    let tiny = Printed::default().needs(
        &section,
        Some(Size {
            width: 120,
            height: 2,
        }),
        now(),
    );
    assert_eq!(
        tiny.lines().count(),
        1,
        "even a two-row pane gets only one section row"
    );
    assert!(
        tiny.contains("20") && tiny.contains("ae list"),
        "tiny header carries count + next step: {tiny}"
    );
}

#[test]
fn needs_view_wide_characters_and_tabs_fit_physical_cell_budget() {
    let section = Ok(Section {
        rows: vec![row(
            "scout",
            "spawned.0",
            Reason::Blocked,
            "中中中中中中中中中中中中\tAFTER",
        )],
    });
    let text = Printed::default().needs(
        &section,
        Some(Size {
            width: 28,
            height: 24,
        }),
        now(),
    );
    assert!(
        text.contains("scout") && text.contains('中'),
        "fixture content reaches clipping assertion: {text}"
    );
    assert!(!text.contains('\t'));
    assert!(text.lines().count() <= 8);
    for line in text.lines() {
        // The fixture's CJK glyph takes two terminal cells; remaining printed
        // ASCII/punctuation glyphs take one. Independent of the production bound.
        let cells: usize = line.chars().map(|ch| if ch == '中' { 2 } else { 1 }).sum();
        assert!(
            cells <= 28,
            "a logical row must not wrap: {cells} cells, {line:?}"
        );
    }
}

#[test]
fn open_grammar_accepts_exact_seat_name_without_messaging_it_and_rejects_damage() {
    let pair = vec!["lead".to_owned(), "colead".to_owned()];
    let accepted = ae::console::input::command(b"/open scout", &pair);
    assert!(
        !matches!(accepted, Command::Refused(_)),
        "valid /open is a local navigation command: {accepted:?}"
    );
    assert!(
        !matches!(accepted, Command::Ask { .. } | Command::Close(_)),
        "opening is neither an ask nor request closure"
    );
    for bad in [
        b"/open".as_slice(),
        b"/open scout extra",
        b"/open other:scout",
        b"/open scout\x1b[2J",
    ] {
        assert!(
            matches!(ae::console::input::command(bad, &pair), Command::Refused(_)),
            "hostile grammar refused: {bad:?}"
        );
    }
    assert!(matches!(
        ae::console::input::command(b"@colead still a message", &pair),
        Command::Ask { .. }
    ));
    assert!(matches!(
        ae::console::input::command(b"@scout no worker messaging", &pair),
        Command::Refused(_)
    ));
}

#[test]
fn open_read_only_input_allows_navigation_but_never_asks_or_closes() {
    let mut input = Input::new(vec!["lead".to_owned(), "colead".to_owned()]);
    input.set_seats(vec![SeatRef {
        slot: "main".to_owned(),
        name: "lead".to_owned(),
    }]);
    let t = Instant::now();
    let _ = input.tick(Reading::NotOwner("another chat owns input".to_owned()), t);
    let effects = input.chunk(b"/open lead\n", t + Duration::from_millis(1));
    assert!(
        effects
            .iter()
            .any(|effect| format!("{effect:?}").starts_with("Open(")),
        "read-only navigation is an effect: {effects:?}"
    );
    for line in [
        b"@lead should-not-send\n".as_slice(),
        b"/close\n",
        b"ordinary-text\n",
    ] {
        let effects = input.chunk(line, t + Duration::from_millis(2));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Ask { .. } | Effect::Close(_)))
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Print(text) if text.contains("read-only")))
        );
    }
    let _ = input.chunk(b"pre-owner-draft", t + Duration::from_millis(3));
    let _ = input.tick(Reading::Owner, t + Duration::from_millis(4));
    let effects = input.chunk(b"\n", t + Duration::from_millis(5));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Ask { .. })),
        "promotion drops pre-ownership draft"
    );
    let effects = input.chunk(b"fresh-owner-line\n", t + Duration::from_millis(6));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Ask { body, .. } if body == "fresh-owner-line"))
    );
}

#[test]
fn open_input_resolves_only_explicit_roster_snapshot_and_carries_slot_with_name() {
    let mut input = Input::new(vec!["lead".to_owned(), "colead".to_owned()]);
    let t = Instant::now();
    let _ = input.tick(Reading::Owner, t);
    let effects = input.chunk(b"/open scout\n", t);
    assert!(
        !effects
            .iter()
            .any(|effect| format!("{effect:?}").starts_with("Open(")),
        "no roster read cannot prove a target"
    );
    assert!(effects.iter().any(|effect| matches!(effect, Effect::Print(text) if text.contains("scout") && text.contains("refused:"))));
    input.set_seats(vec![SeatRef {
        slot: "spawned.7".to_owned(),
        name: "scout".to_owned(),
    }]);
    let effects = input.chunk(b"/open scout\n", t + Duration::from_millis(1));
    let open = effects
        .iter()
        .map(|effect| format!("{effect:?}"))
        .find(|effect| effect.starts_with("Open("));
    assert!(
        open.is_some(),
        "spawned seat is available outside conversation pair: {effects:?}"
    );
    let effect = open.expect("Open assertion passed");
    assert!(
        effect.contains("spawned.7") && effect.contains("scout"),
        "effect carries exact captured SeatRef: {effect}"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Ask { .. } | Effect::Close(_)))
    );
}

#[derive(Clone)]
struct ProofRig {
    meta: Meta,
    seat: SeatRef,
    slots: Vec<ObservedSlot>,
    panes: Vec<WindowPane>,
    members: Vec<PickerPane>,
}

impl ProofRig {
    fn new() -> Self {
        Self {
            meta: Meta::parse("schema=2\nseat.spawned.0=scout\n"),
            seat: SeatRef {
                slot: "spawned.0".to_owned(),
                name: "scout".to_owned(),
            },
            slots: vec![ObservedSlot {
                pane: "%7".to_owned(),
                slot: "spawned.0".to_owned(),
                agent: "scout".to_owned(),
            }],
            panes: vec![WindowPane {
                pane_id: "%7".to_owned(),
                window_id: "@3".to_owned(),
                theme: String::new(),
                reader_src: None,
                console: None,
                dead: false,
                window_index: 3,
                pane_index: 0,
                agent: Some("scout".to_owned()),
            }],
            members: vec![PickerPane {
                session_id: "$1".to_owned(),
                pane: "%7".to_owned(),
            }],
        }
    }

    fn facts(&self) -> Facts<'_> {
        Facts {
            seat: &self.seat,
            roster: self.meta.roster(),
            bound_uuid: UUID,
            session_id: Some("$1"),
            uuid_stamp: Some(UUID),
            slots: Some(&self.slots),
            panes: Some(&self.panes),
            members: Some(&self.members),
        }
    }
}

#[test]
fn open_target_proves_fold_identity_live_pane_uuid_and_membership_without_a_client() {
    let rig = ProofRig::new();
    let proof = open::target(&rig.facts());
    assert!(
        proof.is_ok(),
        "all proof legs present; no client fact needed: {proof:?}"
    );
    assert_eq!(
        proof.expect("proof assertion passed"),
        Target {
            seat: rig.seat.clone(),
            session_id: "$1".to_owned(),
            pane: "%7".to_owned(),
            uuid: UUID.to_owned(),
        }
    );
}

#[test]
fn open_target_refuses_replaced_dead_ambiguous_foreign_and_missing_panes_by_name() {
    let base = ProofRig::new();
    let mut replaced = base.clone();
    replaced.meta = Meta::parse("schema=2\nseat.spawned.0=replacement\n");
    let mut missing = base.clone();
    missing.slots.clear();
    let mut ambiguous = base.clone();
    ambiguous.slots.push(base.slots[0].clone());
    ambiguous.slots[1].pane = "%8".to_owned();
    let mut other = base.clone();
    other.slots[0].agent = "replacement".to_owned();
    let mut dead = base.clone();
    dead.panes[0].dead = true;
    let mut foreign = base.clone();
    foreign.members[0].session_id = "$2".to_owned();
    for (rig, expected) in [
        (replaced, Refusal::Replaced),
        (missing, Refusal::NoPane),
        (ambiguous, Refusal::Ambiguous),
        (other, Refusal::OtherAgent),
        (dead, Refusal::Dead),
        (foreign, Refusal::NotMember),
    ] {
        assert_eq!(
            open::target(&rig.facts()),
            Err(expected.clone()),
            "proof refuses exactly its failed leg"
        );
        let line = expected.line("scout");
        assert!(
            line.contains("refused:") && line.contains("scout"),
            "target refusal names seat: {line}"
        );
    }
    let mut facts = base.facts();
    facts.uuid_stamp = Some(OTHER_UUID);
    assert_eq!(open::target(&facts), Err(Refusal::SessionReplaced));
    facts = base.facts();
    facts.session_id = None;
    assert_eq!(open::target(&facts), Err(Refusal::SessionReplaced));
    facts = base.facts();
    facts.members = None;
    assert!(matches!(open::target(&facts), Err(Refusal::Unread(_))));
}

#[test]
fn open_act_args_guard_identity_and_never_target_a_client_or_type_into_a_pane() {
    let target = Target {
        seat: ProofRig::new().seat,
        session_id: "$1".to_owned(),
        pane: "%7".to_owned(),
        uuid: UUID.to_owned(),
    };
    let server = ServerId::Selected(Selector::Socket(PathBuf::from("/tmp/fixture/tmux.sock")));
    let args = open::select_args(&server, &target);
    assert!(args.is_some(), "proven local target has a select command");
    let args = args.expect("argument assertion passed");
    assert!(
        !args
            .iter()
            .any(|arg| arg == "-c" || arg.contains("switch-client") || arg.contains("send-keys")),
        "session-scoped navigation only: {args:?}"
    );
    let predicate = args
        .iter()
        .find(|arg| arg.contains("#{session_id}"))
        .expect("execution-time membership guard");
    for leg in [
        "#{session_id}",
        "#{@ae_session_uuid}",
        "#{@ae_slot}",
        "#{@ae_agent}",
        "#{pane_dead}",
        UUID,
        "spawned.0",
        "scout",
    ] {
        assert!(predicate.contains(leg), "guard keeps {leg}: {predicate}");
    }
    for broken in ["pane", "session", "slot", "name", "uuid"] {
        let mut damaged = target.clone();
        match broken {
            "pane" => damaged.pane = "%7;select-pane".to_owned(),
            "session" => damaged.session_id = "$x".to_owned(),
            "slot" => damaged.seat.slot = "spawned.x".to_owned(),
            "name" => damaged.seat.name = "scout;next".to_owned(),
            _ => damaged.uuid = "not-a-uuid".to_owned(),
        }
        assert!(
            open::select_args(&server, &damaged).is_none(),
            "invalid {broken} refuses before argv"
        );
    }
    let success = open::outcome(true, "ae-open:selected\n", "scout");
    assert!(success.contains("opened scout") && success.contains("prefix h returns"));
    let refused = open::outcome(true, "ae-open:moved\n", "scout");
    assert!(refused.contains("refused:") && refused.contains("scout"));
    assert!(open::outcome(true, "unknown marker\n", "scout").contains("uncertain"));
}

fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

struct ChatRig {
    scratch: super::cli::OwnedScratch,
    socket: PathBuf,
    dir: PathBuf,
    config: PathBuf,
    scout: String,
}

impl ChatRig {
    fn new(tag: &str) -> Self {
        let mut scratch = super::cli::OwnedScratch::root("needs", tag);
        let socket = scratch.join("sock");
        scratch.add_tmux_server(socket.clone());
        let config = scratch.join("config");
        std::fs::write(&config, "").expect("private config");
        let root = scratch.path();
        let store = root.join("claude");
        let ids = [
            "0199c0de-1234-4890-abcd-ef0123456789",
            "0199c0de-1234-4890-abcd-ef0123456790",
            "0199c0de-1234-4890-abcd-ef0123456791",
        ];
        let mut roster = format!(
            "session_id={UUID}\nmode=local\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\n",
            socket.display()
        );
        for ((slot, name), id) in [
            ("main", "lead"),
            ("worker.0", "colead"),
            ("spawned.0", "scout"),
        ]
        .iter()
        .zip(ids)
        {
            roster.push_str(&super::board::claude_roster(slot, name, id, &store));
            let body = if *name == "scout" {
                "WORKER-TRANSCRIPT-MUST-STAY-HIDDEN"
            } else {
                "LEAD-CONVERSATION-READY"
            };
            super::board::plant_transcript(
                &store,
                "work",
                id,
                &[super::board::user("2026-10-03T17:30:00Z", body)],
            );
        }
        let dir = super::board::plant_session(root, "one", &roster);
        std::fs::write(dir.join("events.jsonl"), concat!(
            "{\"ts\":\"2026-10-03T17:49:00Z\",\"actor\":\"scout\",\"action\":\"chat\",\"summary\":\"WORKER-SAY-MUST-STAY-HIDDEN\"}\n",
            "{\"ts\":\"2026-10-03T17:50:00Z\",\"actor\":\"watchdog\",\"action\":\"human-prompt\",\"target\":\"scout\",\"summary\":\"SCOUT-TRUST-MODAL\"}\n",
            "{\"ts\":\"2026-10-03T17:51:00Z\",\"actor\":\"lead\",\"action\":\"state\",\"ref\":\"waiting-user\",\"summary\":\"LEAD-FULL-QUESTION\"}\n",
        )).expect("synthetic journal");
        ae::watchdog_glue::touch_beat(&dir).expect("fresh private watchdog evidence");
        let mut rig = Self {
            scratch,
            socket,
            dir,
            config,
            scout: String::new(),
        };
        let lead = rig
            .tmux(&[
                "new-session",
                "-d",
                "-s",
                "one",
                "-x",
                "120",
                "-y",
                "40",
                "-P",
                "-F",
                "#{pane_id}",
                "sleep",
                "600",
            ])
            .trim()
            .to_owned();
        rig.stamp(&lead, "main", "lead");
        let colead = rig.pane("colead");
        rig.stamp(&colead, "worker.0", "colead");
        rig.scout = rig.pane("scout");
        rig.stamp(&rig.scout, "spawned.0", "scout");
        rig.tmux(&["set-option", "-t", "one", "@ae_session_uuid", UUID]);
        rig.tmux(&["set-option", "-t", "one", "@ae_main_pane", &lead]);
        rig.tmux(&["set-option", "-t", "one", "@ae_look", "off"]);
        rig.tmux(&["set-option", "-t", "one", "status", "off"]);
        rig
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, text) = super::phase2::run_tmux(&args, self.scratch.path());
        assert!(ok, "private tmux {tail:?}: {text}");
        text
    }

    fn pane(&self, name: &str) -> String {
        self.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "one:",
            "-n",
            name,
            "sleep",
            "600",
        ])
        .trim()
        .to_owned()
    }

    fn stamp(&self, pane: &str, slot: &str, name: &str) {
        self.tmux(&["set-option", "-p", "-t", pane, "@ae_slot", slot]);
        self.tmux(&["set-option", "-p", "-t", pane, "@ae_agent", name]);
    }

    fn command(&self) -> super::cli::Runner {
        let mut command = super::cli::ae();
        command
            .env("HOME", self.scratch.path())
            .env("AE_HOME", self.scratch.path())
            .env("CONFIG_FILE", &self.config)
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME");
        command
    }

    fn once(&self) -> String {
        let mut command = self.command();
        let child = command
            .args(["chat", "one", "--all"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("chat fixture starts");
        let output =
            super::cli::bounded(child, LIMIT).expect("one-shot chat exits within fixture limit");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("UTF-8 chat")
    }

    fn chat(&self, read_only: bool) -> String {
        if read_only {
            let owner = self.pane("earlier-console");
            self.tmux(&["set-option", "-p", "-t", &owner, "@ae_console", UUID]);
        }
        let command = format!(
            "stty -icanon -echo; exec env -u CLAUDE_CONFIG_DIR -u CODEX_HOME HOME={} AE_HOME={} CONFIG_FILE={} AE_TMUX_SERVER_KIND=socket AE_TMUX_SERVER={} {} chat one --follow --input",
            quoted(&self.scratch.display().to_string()),
            quoted(&self.scratch.display().to_string()),
            quoted(&self.config.display().to_string()),
            quoted(&self.socket.display().to_string()),
            quoted(env!("CARGO_BIN_EXE_ae")),
        );
        let chat = self
            .tmux(&[
                "new-window",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-t",
                "one:",
                "-n",
                "chat",
                &command,
            ])
            .trim()
            .to_owned();
        self.tmux(&["set-option", "-p", "-t", &chat, "@ae_console", UUID]);
        self.tmux(&["select-window", "-t", &chat]);
        self.tmux(&["select-pane", "-t", &chat]);
        let ready = Self::wait(|| {
            let screen = self.screen(&chat);
            screen.contains("LEAD-CONVERSATION-READY")
                && if read_only {
                    screen.contains("read-only:")
                } else {
                    screen.contains("to lead>")
                }
        });
        assert!(
            ready,
            "chat fixture reaches ownership readiness: {}",
            self.screen(&chat)
        );
        chat
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-S", "-", "-t", pane])
    }

    fn current(&self) -> String {
        self.tmux(&["display-message", "-p", "-t", "one", "#{pane_id}"])
            .trim()
            .to_owned()
    }

    fn wait(mut condition: impl FnMut() -> bool) -> bool {
        let until = Instant::now() + LIMIT;
        loop {
            if condition() {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn type_line(&self, pane: &str, line: &str) {
        // Fixture input only, on this rig's private socket. No agent delivery.
        self.tmux(&["send-keys", "-t", pane, "-l", "--", line]);
        self.tmux(&["send-keys", "-t", pane, "Enter"]);
    }

    fn journal(&self) -> Vec<u8> {
        std::fs::read(self.dir.join("events.jsonl")).expect("private journal")
    }

    fn server(&self) -> ServerId {
        ServerId::Selected(Selector::Socket(self.socket.clone()))
    }
}

#[test]
fn needs_chat_cli_shows_spawned_prompt_and_keeps_conversation_lead_pair_only() {
    let rig = ChatRig::new("pipe");
    let journal = std::fs::read_to_string(rig.dir.join("events.jsonl")).expect("private journal");
    let events = journal
        .lines()
        .map(|line| Event::parse_line(line).expect("fixture event"))
        .collect();
    let classifier = FoldRig::new(
        &[
            ("main", "lead"),
            ("worker.0", "colead"),
            ("spawned.0", "scout"),
        ],
        events,
    );
    assert_eq!(
        classifier
            .entry
            .agents
            .iter()
            .find(|agent| agent.name == "scout")
            .expect("scout fixture")
            .reason,
        Some(Reason::Blocked),
        "fixture leaves human prompt standing in the existing classifier"
    );
    let text = rig.once();
    assert!(
        text.contains("LEAD-CONVERSATION-READY") && text.contains("LEAD-FULL-QUESTION"),
        "baseline conversation fixture ready: {text}"
    );
    assert!(
        !text.contains("WORKER-TRANSCRIPT-MUST-STAY-HIDDEN")
            && !text.contains("WORKER-SAY-MUST-STAY-HIDDEN")
    );
    let section = text.find("needs you").map(|at| &text[at..]);
    assert!(
        section.is_some(),
        "shipped chat must wire the all-roster section: {text}"
    );
    let section = section.expect("section assertion passed");
    assert!(
        section.contains("scout")
            && section.contains("SCOUT-TRUST-MODAL")
            && section.contains("human prompt"),
        "spawned prompt shows actual source: {section}"
    );
    assert!(
        section.contains("lead pair") && section.contains("since") && section.contains("as of")
    );
    assert!(section.contains("/open scout"));
    assert!(!text.contains('\x1b'), "pipe is plain bytes");
}

#[test]
fn needs_chat_cli_collapses_watchdog_off_into_one_named_gap() {
    let rig = ChatRig::new("gap-pipe");
    std::fs::write(rig.dir.join("events.jsonl"), "").expect("clear synthetic reasons");
    std::fs::remove_file(ae::watchdog_glue::beat_path(&rig.dir)).expect("private watchdog off");
    let text = rig.once();
    assert!(
        text.contains("LEAD-CONVERSATION-READY"),
        "conversation fixture ready: {text}"
    );
    let gaps: Vec<_> = text
        .lines()
        .filter(|line| {
            line.contains("watchdog") && line.contains("unverified") && line.contains("3 seats")
        })
        .collect();
    assert_eq!(
        gaps.len(),
        1,
        "CLI consumes the view's shared-cause collapse: {text}"
    );
    for name in ["lead", "colead", "scout"] {
        assert!(
            gaps[0].contains(name),
            "gap names every seat while width permits"
        );
    }
    assert!(!text.contains("nothing standing"));
}

#[test]
fn open_chat_input_selects_session_for_two_viewers_without_typing_or_journal_write() {
    let rig = ChatRig::new("open-owner");
    let chat = rig.chat(false);
    let _first =
        super::cli::tmux_attached_client(&rig.socket, "one").expect("first private client");
    let _second =
        super::cli::tmux_attached_client(&rig.socket, "one").expect("second private client");
    assert!(
        ChatRig::wait(|| rig
            .tmux(&["list-clients", "-F", "#{client_name}"])
            .lines()
            .count()
            == 2),
        "two real clients attached"
    );
    let journal = rig.journal();
    let target_before = rig.screen(&rig.scout);
    rig.type_line(&chat, "/open scout");
    let opened =
        ChatRig::wait(|| rig.current() == rig.scout || rig.screen(&chat).contains("refused:"));
    assert!(
        opened && rig.current() == rig.scout,
        "typed navigation selects target after readiness: {}",
        rig.screen(&chat)
    );
    let clients = rig.tmux(&["list-clients", "-F", "#{pane_id}"]);
    assert_eq!(
        clients.lines().filter(|pane| *pane == rig.scout).count(),
        2,
        "both session viewers follow: {clients}"
    );
    assert!(
        ChatRig::wait(|| rig.screen(&chat).contains("opened scout")
            && rig.screen(&chat).contains("prefix h returns")),
        "success names return path"
    );
    assert_eq!(
        rig.journal(),
        journal,
        "navigation writes no request or approval"
    );
    assert_eq!(
        rig.screen(&rig.scout),
        target_before,
        "target receives no typed bytes"
    );
}

#[test]
fn open_chat_non_owner_still_navigates_and_unknown_target_refusal_names_target() {
    let rig = ChatRig::new("open-reader");
    let chat = rig.chat(true);
    let journal = rig.journal();
    rig.type_line(&chat, "/open missing-seat");
    let named = ChatRig::wait(|| {
        rig.screen(&chat)
            .lines()
            .any(|line| line.contains("refused:") && line.contains("missing-seat"))
    });
    assert!(
        named,
        "unknown-target refusal names seat after read-only readiness: {}",
        rig.screen(&chat)
    );
    assert_eq!(rig.current(), chat, "failed proof leaves session at chat");
    rig.type_line(&chat, "/open scout");
    assert!(
        ChatRig::wait(|| rig.current() == rig.scout),
        "non-owner chat navigation works: {}",
        rig.screen(&chat)
    );
    assert_eq!(rig.journal(), journal);
}

#[test]
fn open_execution_rechecks_replaced_seat_uuid_membership_and_dead_pane() {
    let rig = ChatRig::new("open-race");
    let chat = rig.pane("chat-anchor");
    let target = Target {
        seat: SeatRef {
            slot: "spawned.0".to_owned(),
            name: "scout".to_owned(),
        },
        session_id: rig
            .tmux(&["display-message", "-p", "-t", "one", "#{session_id}"])
            .trim()
            .to_owned(),
        pane: rig.scout.clone(),
        uuid: UUID.to_owned(),
    };
    let args = open::select_args(&rig.server(), &target);
    assert!(args.is_some(), "captured target creates guarded act");
    let args = args.expect("argument assertion passed");
    assert!(
        !args
            .iter()
            .any(|arg| arg == "-c" || arg.contains("switch-client"))
    );
    for (scope, option, bad, restore) in [
        ("pane", "@ae_agent", "replacement", "scout"),
        ("pane", "@ae_slot", "spawned.99", "spawned.0"),
        ("session", "@ae_session_uuid", OTHER_UUID, UUID),
    ] {
        rig.tmux(&["select-window", "-t", &chat]);
        let target_scope = if scope == "pane" {
            rig.scout.as_str()
        } else {
            "one"
        };
        if scope == "pane" {
            rig.tmux(&["set-option", "-p", "-t", target_scope, option, bad]);
        } else {
            rig.tmux(&["set-option", "-t", target_scope, option, bad]);
        }
        let (ok, output) = super::phase2::run_tmux(&args, rig.scratch.path());
        assert!(ok, "guard evaluates against changed {option}: {output}");
        assert_eq!(
            rig.current(),
            chat,
            "changed {option} cannot select replacement"
        );
        assert!(
            output.contains("ae-open:moved"),
            "guard failure is confirmed, not guessed: {output}"
        );
        if scope == "pane" {
            rig.tmux(&["set-option", "-p", "-t", target_scope, option, restore]);
        } else {
            rig.tmux(&["set-option", "-t", target_scope, option, restore]);
        }
    }
    rig.tmux(&["new-session", "-d", "-s", "foreign", "sleep", "600"]);
    rig.tmux(&["move-window", "-s", &rig.scout, "-t", "foreign:"]);
    let (ok, output) = super::phase2::run_tmux(&args, rig.scratch.path());
    assert!(ok && output.contains("ae-open:moved"));
    assert_eq!(
        rig.current(),
        chat,
        "foreign membership fails execution guard"
    );
    rig.tmux(&["move-window", "-s", &rig.scout, "-t", "one:"]);
    rig.tmux(&[
        "set-window-option",
        "-t",
        &rig.scout,
        "remain-on-exit",
        "on",
    ]);
    rig.tmux(&["respawn-pane", "-k", "-t", &rig.scout, "sh", "-c", "exit 0"]);
    assert!(
        ChatRig::wait(|| rig
            .tmux(&["display-message", "-p", "-t", &rig.scout, "#{pane_dead}"])
            .trim()
            == "1"),
        "private dead-pane fixture settles"
    );
    let (ok, output) = super::phase2::run_tmux(&args, rig.scratch.path());
    assert!(ok && output.contains("ae-open:moved"));
    assert_eq!(rig.current(), chat, "dead pane fails execution guard");
}
