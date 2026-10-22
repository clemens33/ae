//! Frozen honestwords acceptance: #237 item 8, contract-r2 and lead's Idle ruling.
//! Existing public renderers only; synthetic journals; no live session reads.

#![allow(
    clippy::expect_used,
    reason = "acceptance builds synthetic journals and reads rendered terminal cells"
)]

use ae::app::draw::{Composer, Screen, draw};
use ae::app::fleet::{Counts, Facts, Fleet, Line2, Row};
use ae::app::model::Model;
use ae::app::overview;
use ae::console::lane::Lane;
use ae::digest::{SessionEntry, Status};
use ae::events::{Cursor, Drain, Event, SessionKey};
use ae::harness_state::HarnessState;
use ae::json::Value;
use ae::meta::Meta;
use ae::orchestrator::PickerBounds;
use ae::session::{AgentRuntime, MetaRead, RecordSnapshot, SessionRead, SessionRuntime};
use ae::theme::{self, FleetOrder, Look, Mark, Palette};
use ae::time::Timestamp;
use ae::tmux::{PickerAgent, PickerSession};
use ae::watchdog::QuietKind;
use ae::watchdog_daemon::Verdict;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

const NOW: i64 = 1_792_483_200;
const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";

// Independent oracle: contract-r2 sha256 d0bd51430e92169c0a4ad8d4c12f00950bff8e3028f60be59f375656c1bc861d.
// Independent of Mark::word and Verdict::reason.
const WORDS: [(&str, &str, Mark); 14] = [
    ("working", "working", Mark::Working),
    ("sweeping", "working (sweeping)", Mark::Working),
    ("done", "finished", Mark::Done),
    ("waiting-agent", "waiting on an agent", Mark::WaitingAgent),
    ("waiting-user", "needs you (waiting on you)", Mark::NeedsYou),
    ("blocked", "needs you (blocked)", Mark::NeedsYou),
    ("throttled", "needs you (throttled)", Mark::NeedsYou),
    ("limit", "needs you (usage limit)", Mark::NeedsYou),
    ("prompt", "needs you (prompt)", Mark::NeedsYou),
    ("wedged", "needs you (sweep wedged)", Mark::NeedsYou),
    ("stale", "unknown", Mark::Stale),
    ("starting", "unknown (starting)", Mark::Stale),
    ("dead", "dead", Mark::Dead),
    ("idle", "no current status", Mark::Idle),
];

fn now() -> Timestamp {
    Timestamp::from_epoch(NOW)
}

fn picker_agent(state: &str) -> PickerAgent {
    PickerAgent {
        name: "lead".to_owned(),
        profile: "demo".to_owned(),
        state: state.to_owned(),
        pane: "%1".to_owned(),
        client: String::new(),
        model: String::new(),
        effort: String::new(),
        drift: false,
    }
}

fn declaration(state: &str, reason: &str, age: i64, actor: &str) -> Event {
    Event::parse_line(
        &Value::Obj(vec![
            (
                "ts".to_owned(),
                Value::str(Timestamp::from_epoch(NOW - age).to_string()),
            ),
            ("actor".to_owned(), Value::str(actor)),
            ("actor_slot".to_owned(), Value::str("main")),
            ("actor_session".to_owned(), Value::str("one")),
            ("actor_session_id".to_owned(), Value::str(UUID)),
            ("action".to_owned(), Value::str("state")),
            ("ref".to_owned(), Value::str(state)),
            ("summary".to_owned(), Value::str(reason)),
        ])
        .render(),
    )
    .expect("synthetic declaration")
}

fn entry(events: Option<Vec<Event>>) -> SessionEntry {
    let snapshot = RecordSnapshot {
        meta: Some(Meta::parse(&format!(
            "schema=2\nsession_id={UUID}\nlayout=lead-pair\nseat.main=lead\nprofile.main=demo\n"
        ))),
        meta_read: MetaRead::Parsed,
        events: events.map(|events| {
            SessionRead::from_drain_in(
                &Drain {
                    events,
                    cursor: Cursor::default(),
                    skipped: Vec::new(),
                    drained: true,
                },
                SessionKey::new("one", UUID),
            )
        }),
        legacy_created_epoch: None,
    };
    let runtime = SessionRuntime {
        status: Status::Running,
        branch: None,
        agents: vec![AgentRuntime {
            slot: "main".to_owned(),
            alive: Some(true),
            alert: None,
            observed: HarnessState::Idle,
        }],
    };
    ae::session::entry_from(&snapshot, "one", &runtime, now(), 1800)
}

fn declared(state: &str, reason: &str) -> SessionEntry {
    entry(Some(vec![declaration(state, reason, 720, "lead")]))
}

fn picker_row(state: &str, icons: bool, grammar: &str, width: usize) -> String {
    let extra = if grammar == "v2" { "::::" } else { "" };
    let session = PickerSession {
        name: "one".to_owned(),
        id: "$1".to_owned(),
        rank: 2,
        glyph: String::new(),
        main_pane: "%1".to_owned(),
        branch: String::new(),
        agents: format!("{grammar};{NOW};60;lead:demo:{state}:%1{extra}"),
        activity: Some(NOW),
        created_at: Some(NOW),
        goal: String::new(),
    };
    let menu = ae::orchestrator::menu_for_client_session_in(
        &[session],
        &[],
        &[],
        icons,
        &Palette::DARCULA,
        Some("client"),
        Some("$1"),
        None,
        PickerBounds {
            width,
            height: 20,
            now_epoch: NOW,
        },
        &FleetOrder::EMPTY,
    )
    .expect("room for synthetic picker");
    menu.items
        .into_iter()
        .find(|item| {
            item.label
                .trim_start()
                .starts_with(&format!("{} lead", picker_agent(state).mark().glyph(icons)))
        })
        .expect("expanded lead row")
        .label
}

fn app_sidebar(entry: &SessionEntry, state: &str, icons: bool) -> String {
    let mark = picker_agent(state).mark();
    let fleet = Fleet {
        rows: vec![Row {
            name: "one".to_owned(),
            index: 1,
            mark,
            needy: false,
            unanswered: None,
            counts: Counts::Marks(vec![(mark, false, 1)]),
            line2: Line2::NoGoal,
            home: true,
        }],
        home: Some("one".to_owned()),
    };
    let model = Model::new(&fleet);
    let overview = overview::of(entry, None, None, now());
    let facts = Facts::Seats {
        id: "$1".to_owned(),
        agents: vec![picker_agent(state)],
    };
    let lane = Lane::default();
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(entry),
        pair: &[],
        agents: Some(&facts),
        lane: &lane,
        composer: Composer::Home {
            home: "one",
            view: None,
            draft: "",
        },
        look: Some(Look::read(
            if icons { "on" } else { "off" },
            "darcula",
            "on",
            "off",
        )),
        zone: None,
        now: now(),
    };
    // With no dragged split, 220 columns give a 44-cell sidebar and 40-cell
    // seat room (44 - 2 - LEFT). Mark, gap, lead, gap and the longest word
    // need 2 + 4 + 1 + 26 = 33 cells. Empty model/effort adds no label row.
    let mut buf = Buffer::empty(Rect::new(0, 0, 220, 60));
    draw(&screen, &mut buf);
    (0..60)
        .map(|y| (0..44).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn honestwords_picker_draws_approved_words_from_both_roster_grammars() {
    for (wire, human, mark) in WORDS {
        for icons in [true, false] {
            for grammar in ["v1", "v2"] {
                let row = picker_row(wire, icons, grammar, 160);
                assert!(row.contains(human), "{wire}/{grammar}: {row}");
                assert!(
                    row.trim_start().starts_with(mark.glyph(icons)),
                    "{wire}: {row}"
                );
            }
        }
    }
}

#[test]
fn honestwords_app_draws_approved_words_without_inferring_done_from_idle() {
    let entry = entry(Some(Vec::new()));
    for (wire, human, mark) in WORDS {
        for icons in [true, false] {
            let text = app_sidebar(&entry, wire, icons);
            assert!(
                text.lines()
                    .any(|row| row.contains("lead") && row.contains(human)),
                "{wire}: {text}"
            );
            assert!(text.contains(mark.glyph(icons)), "{wire}: {text}");
            if wire == "idle" {
                assert!(
                    !text.contains("finished"),
                    "empty composer never means completion"
                );
            }
        }
    }
    let text = app_sidebar(&declared("working", "still checking"), "idle", true);
    assert!(
        text.contains("no current status"),
        "Idle may have a working declaration: {text}"
    );
    assert!(
        !text.contains("no status declared"),
        "do not claim absence: {text}"
    );
}

#[test]
fn honestwords_border_builders_translate_stable_verdict_words() {
    let verdicts = [
        Verdict::Active,
        Verdict::Meta(ae::watchdog::SweepVerdict::MetaSweeping),
        Verdict::Quiet(QuietKind::Done),
        Verdict::Quiet(QuietKind::WaitingAgent),
        Verdict::Quiet(QuietKind::WaitingUser),
        Verdict::Quiet(QuietKind::Blocked),
        Verdict::Throttled,
        Verdict::Limit,
        Verdict::HumanPrompt,
        Verdict::Meta(ae::watchdog::SweepVerdict::MetaWedged),
        Verdict::Stale,
        Verdict::Meta(ae::watchdog::SweepVerdict::MetaStarting),
        Verdict::Dead,
        Verdict::Idle,
    ];
    for (verdict, (wire, human, mark)) in verdicts.into_iter().zip(WORDS) {
        assert_eq!(
            verdict.reason(),
            wire,
            "published verdict words stay compatible"
        );
        assert_eq!(verdict.mark(), mark, "wording cannot alter decisions");
        for icons in [true, false] {
            let text =
                theme::pane_state(&Palette::DARCULA, mark, mark.glyph(icons), verdict.reason());
            assert!(text.ends_with(&format!(" {human}")), "{wire}: {text}");
        }
    }
    for icons in [true, false] {
        let frame = theme::working_frame(3, &Palette::DARCULA, icons);
        let text = theme::pane_state_frame(&frame, "sweeping");
        assert!(
            text.ends_with(" working (sweeping)"),
            "ticker preserves sweep distinction: {text}"
        );
    }
}

#[test]
fn honestwords_list_labels_declarations_and_observation_independently() {
    for (wire, human) in [
        ("working", "working"),
        ("done", "finished"),
        ("waiting-agent", "waiting on an agent"),
        ("waiting-user", "needs you (waiting on you)"),
        ("blocked", "needs you (blocked)"),
    ] {
        let entry = declared(wire, "declared explanation");
        let text = ae::listing::table_at(&[&entry], now());
        assert!(text.contains(human), "{wire}: {text}");
        assert!(
            text.contains("observed:input ready"),
            "observed fact is independent: {text}"
        );
        assert!(!text.contains("observed:idle"), "{text}");
        let json = entry.agents[0].to_json();
        assert_eq!(json.get("state"), Some(&Value::str(wire)));
        assert_eq!(json.get("observed"), Some(&Value::str("idle")));
        let Value::Obj(fields) = json else {
            panic!("agent object")
        };
        assert_eq!(
            fields
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            [
                "ref",
                "alias",
                "name",
                "session_id",
                "alive",
                "observed",
                "state",
                "reason"
            ]
        );
    }
    let bare = entry(Some(Vec::new()));
    let text = ae::listing::table_at(&[&bare], now());
    assert!(text.contains("no current status"), "{text}");
    assert!(!text.contains("finished"), "{text}");
}

#[test]
fn honestwords_declared_reason_and_its_age_reach_app_and_list() {
    for state in [
        "working",
        "waiting-agent",
        "blocked",
        "waiting-user",
        "done",
    ] {
        let entry = declared(state, "colimafix result");
        for text in [
            ae::listing::table_at(&[&entry], now()),
            app_sidebar(&entry, state, true),
        ] {
            assert!(text.contains("colimafix result"), "{state}: {text}");
            assert!(
                text.contains("12m"),
                "declaration age, not activity age: {text}"
            );
        }
    }
}

#[test]
fn honestwords_reason_lookup_keeps_the_established_slot_routing() {
    let entry = entry(Some(vec![declaration(
        "done",
        "renamed actor reason",
        720,
        "oldlead",
    )]));
    for text in [
        ae::listing::table_at(&[&entry], now()),
        app_sidebar(&entry, "done", true),
    ] {
        assert!(
            text.contains("renamed actor reason"),
            "same slot despite display rename: {text}"
        );
        assert!(text.contains("12m"), "{text}");
    }
}

#[test]
fn honestwords_missing_reason_is_explicit_without_fabricated_age() {
    for entry in [declared("done", ""), entry(Some(Vec::new()))] {
        let state = if entry.agents[0].state.is_some() {
            "done"
        } else {
            "idle"
        };
        for text in [
            ae::listing::table_at(&[&entry], now()),
            app_sidebar(&entry, state, true),
        ] {
            assert!(text.contains("no reason given"), "{text}");
            if state == "idle" {
                assert!(
                    !text.contains("12m"),
                    "no declaration supplies no age: {text}"
                );
            }
        }
    }
}

#[test]
fn honestwords_unreadable_reason_stays_unknown_instead_of_absent() {
    let entry = entry(None);
    for text in [
        ae::listing::table_at(&[&entry], now()),
        app_sidebar(&entry, "stale", true),
    ] {
        assert!(text.contains("unknown"), "{text}");
        assert!(
            text.contains("reason unavailable"),
            "unknown is not missing: {text}"
        );
        assert!(!text.contains("no reason given"), "{text}");
    }
}

#[test]
fn honestwords_finished_keeps_confirmation_and_lapsed_qualifiers() {
    let mut entry = declared("done", "review complete");
    entry.agents[0].done_progress = Some(ae::watchdog::DoneProgress::Lapsed {
        confirmations: 1,
        required: 3,
    });
    let text = ae::listing::table_at(&[&entry], now());
    assert!(
        text.contains("finished (unconfirmed 1/3, lapsed)"),
        "{text}"
    );
    let mut entry = declared("waiting-agent", "worker reply");
    entry.agents[0].wait_progress = Some(ae::watchdog::WaitProgress::Provisional {
        confirmations: 1,
        required: 3,
    });
    let text = ae::listing::table_at(&[&entry], now());
    assert!(
        text.contains("waiting on an agent (unconfirmed 1/3)"),
        "{text}"
    );
}

#[test]
fn honestwords_human_rendering_does_not_change_protocol_glyphs_ranks_or_words() {
    for (mark, wire, rank) in [
        (Mark::Dead, "dead", 5),
        (Mark::NeedsYou, "needs-you", 4),
        (Mark::Working, "working", 2),
        (Mark::WaitingAgent, "waiting-agent", 2),
        (Mark::Done, "done", 1),
        (Mark::Stale, "stale", 3),
        (Mark::Idle, "idle", 0),
    ] {
        assert_eq!(mark.word(), wire);
        assert_eq!(mark.rank(), rank);
    }
    assert_eq!(Mark::from_rank_value(2), Mark::Working);
    assert_eq!(HarnessState::Idle.as_str(), "idle");
    for grammar in ["v1", "v2"] {
        for (wire, _, mark) in WORDS {
            let extra = if grammar == "v2" { "::::" } else { "" };
            let raw = format!("{grammar};{NOW};60;lead:demo:{wire}:%1{extra}");
            let parsed =
                ae::tmux::parse_picker_agents(&raw, NOW).expect("unchanged roster grammar");
            assert_eq!(parsed[0].state, wire);
            assert_eq!(parsed[0].mark(), mark);
        }
    }
}

#[test]
fn honestwords_picker_long_words_stay_inside_narrow_client_bounds() {
    for width in [40, 60, 80, 100] {
        for (wire, _, _) in WORDS {
            let row = picker_row(wire, true, "v2", width);
            assert!(row.chars().count() <= width - 4, "client {width}: {row}");
        }
    }
}

#[test]
fn honestwords_usage_limit_keeps_declared_reason_age_and_reseat_qualifier() {
    let mut entry = declared("working", "checking provider migration");
    entry.agents[0].reason = Some(ae::attention::Reason::Limit);
    for moving in [false, true] {
        entry.agents[0].auto_reseat_open = moving;
        let text = ae::listing::table_at(&[&entry], now());
        let row = text
            .lines()
            .find(|row| row.trim_start().starts_with("lead "))
            .expect("declared seat row");
        assert!(row.contains("needs you (usage limit)"), "{row}");
        assert!(
            row.contains("checking provider migration") && row.contains("12m"),
            "limit keeps the declaration detail: {row}"
        );
        assert_eq!(row.contains("auto-reseat in flight"), moving, "{row}");
    }
}
