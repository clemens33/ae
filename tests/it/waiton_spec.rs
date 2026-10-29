//! Frozen waiton acceptance: #237 owner chain + approved contract-r2.
//! Oracle is literal owner behavior; synthetic journals, existing public readers.

#![allow(
    clippy::expect_used,
    reason = "acceptance fixtures must parse and render"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ae::app::draw::{Composer, Screen, draw};
use ae::app::fleet::{self, Facts};
use ae::app::model::Model;
use ae::app::overview;
use ae::attention::Reason;
use ae::console::lane::Lane;
use ae::console::needs::{self, Inputs, Section};
use ae::digest::{SessionEntry, Status};
use ae::events::{Cursor, Drain, Event, SessionKey};
use ae::harness_state::HarnessState;
use ae::json::Value;
use ae::listing::World;
use ae::meta::Meta;
use ae::session::{AgentRuntime, MetaRead, RecordSnapshot, SessionRead, SessionRuntime};
use ae::state::{self, Command};
use ae::theme::{FleetOrder, Look, Mark};
use ae::time::Timestamp;
use ae::tmux::{Evidence, PickerAgent};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const SEATS: [(&str, &str); 3] = [
    ("main", "lead"),
    ("worker.0", "colead"),
    ("spawned.0", "colimafix"),
];
const WHY: &str = "lead: waiting for owner to finish the migration containers before rollout; evidence and next step: .local/waiton.md";

fn now() -> Timestamp {
    Timestamp::parse("2026-10-21T12:00:00Z").expect("fixed oracle clock")
}

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|word| (*word).to_owned()).collect()
}

fn event(slot: &str, state: &str, age: i64, extra: &str) -> Event {
    let (_, name) = SEATS
        .iter()
        .find(|(key, _)| *key == slot)
        .expect("known seat");
    Event::parse_line(&format!(
        r#"{{"ts":"{}","actor":"{name}","actor_slot":"{slot}","actor_session":"mac","actor_session_id":"{UUID}","action":"state","ref":"{state}","summary":"{WHY}"{extra}}}"#,
        Timestamp::from_epoch(now().epoch() - age),
    ))
    .expect("synthetic state, including lenient wait_on damage")
}

fn fixture(events: Vec<Event>) -> (RecordSnapshot, SessionRuntime) {
    let mut roster = String::new();
    for (slot, name) in SEATS {
        writeln!(
            roster,
            "seat.{slot}={name}\nprofile.{slot}=demo\nagent_bin.{slot}=codex"
        )
        .expect("writing a String");
    }
    let snapshot = RecordSnapshot {
        meta: Some(Meta::parse(&format!(
            "schema=2\nmode=local\nsession=mac\nsession_id={UUID}\nlayout=lead-pair\nidle_nudge_secs=300\ndone_confirmations=2\n{roster}"
        ))),
        meta_read: MetaRead::Parsed,
        events: Some(SessionRead::from_drain_in(
            &Drain {
                events,
                cursor: Cursor::default(),
                skipped: Vec::new(),
                drained: true,
            },
            SessionKey::new("mac", UUID),
        )),
        legacy_created_epoch: None,
    };
    let runtime = SessionRuntime {
        agents: SEATS
            .iter()
            .map(|(slot, _)| AgentRuntime {
                pane_id: None,
                slot: (*slot).to_owned(),
                alive: Some(true),
                alert: None,
                observed: HarnessState::Idle,
            })
            .collect(),
        ..SessionRuntime::new(Status::Running)
    };
    (snapshot, runtime)
}

fn read(snapshot: &RecordSnapshot, runtime: &SessionRuntime) -> SessionEntry {
    ae::session::entry_from(snapshot, "mac", runtime, now(), 1800)
}

fn chain(age: i64, local_target: &str, worker_target: &str) -> (RecordSnapshot, SessionRuntime) {
    fixture(vec![
        event(
            "main",
            "waiting-agent",
            age,
            &format!(r#","wait_on":{local_target}"#),
        ),
        event("worker.0", "done", age, ""),
        event(
            "spawned.0",
            "waiting-agent",
            age,
            &format!(r#","wait_on":{worker_target}"#),
        ),
    ])
}

fn owner_chain(age: i64) -> (RecordSnapshot, SessionRuntime) {
    chain(age, r#""colimafix""#, r#""@owner""#)
}

fn section(snapshot: &RecordSnapshot, runtime: &SessionRuntime, entry: &SessionEntry) -> Section {
    needs::fold(&Inputs {
        session: "mac",
        snapshot,
        entry,
        runtime,
        runtime_read: true,
        beat: Evidence::At(now().epoch()),
        lead_pair: true,
        now: now(),
    })
    .expect("complete fixture sources")
}

fn view_world(entry: SessionEntry, owner_present: bool) -> World {
    let mut entries = vec![entry];
    if owner_present {
        entries.push(SessionEntry::new("owner", Status::Running));
    }
    World::new(now(), entries)
}

fn facts(world: &World) -> BTreeMap<String, Facts> {
    world
        .sessions
        .iter()
        .map(|entry| {
            let agents = entry
                .agents
                .iter()
                .enumerate()
                .map(|(at, agent)| PickerAgent {
                    name: agent.name.clone(),
                    profile: "demo".to_owned(),
                    state: agent.reason.map_or_else(
                        || agent.state.clone().unwrap_or_else(|| "idle".to_owned()),
                        |reason| reason.as_str().to_owned(),
                    ),
                    pane: format!("%{}", at + 1),
                    client: String::new(),
                    model: String::new(),
                    effort: String::new(),
                    drift: false,
                })
                .collect();
            (
                entry.name.clone(),
                Facts::Seats {
                    id: format!("${}", entry.name),
                    agents,
                },
            )
        })
        .collect()
}

fn app(world: &World) -> (Mark, String) {
    let facts = facts(world);
    let last_live = BTreeMap::new();
    let needs = BTreeMap::new();
    let fleet = fleet::rows(
        world,
        &facts,
        &last_live,
        &FleetOrder::EMPTY,
        &needs,
        Some("mac"),
        now(),
    );
    let mark = fleet
        .rows
        .iter()
        .find(|row| row.name == "mac")
        .expect("local row")
        .mark;
    let entry = world
        .sessions
        .iter()
        .find(|entry| entry.name == "mac")
        .expect("local entry");
    let model = Model::new(&fleet);
    let overview = overview::of(entry, None, None, now());
    let lane = Lane::default();
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(entry),
        pair: &[],
        agents: facts.get("mac"),
        lane: &lane,
        composer: Composer::Home {
            home: "mac",
            view: None,
            draft: "",
        },
        look: Some(Look::read("on", "darcula", "on", "off")),
        zone: None,
        now: now(),
    };
    // Render the complete app frame with the default sidebar width.
    let mut buf = Buffer::empty(Rect::new(0, 0, 500, 60));
    draw(&screen, &mut buf);
    let text = (0..60)
        .map(|y| (0..500).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    (mark, text)
}

fn list(world: &World) -> String {
    ae::listing::table_at(&world.sessions.iter().collect::<Vec<_>>(), now())
}

#[test]
fn waiton_cli_records_one_optional_field_and_keeps_reason_separate() {
    for target in ["colimafix", "owner:lead", "@owner", "mac:colimafix"] {
        let Command::Declare(declaration) =
            state::parse(&words(&["waiting-agent", "--on", target, WHY]))
                .expect("grammar-only target accepted")
        else {
            panic!("a declaration")
        };
        let body = state::event_body(now(), "lead", &declaration);
        let json = ae::json::parse(body.trim()).expect("one state record");
        assert_eq!(json.get_str("wait_on"), Some(target), "{body}");
        assert_eq!(
            json.get_str("summary"),
            Some(WHY),
            "option never enters reason"
        );
        assert_eq!(json.get_str("ref"), Some("waiting-agent"));
        assert_eq!(body.lines().count(), 1);
    }
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "assert no journal in this test's owned scratch directory"
)]
fn waiton_cli_refuses_bad_targets_or_other_states_before_any_journal_write() {
    let scratch = super::cli::OwnedScratch::root("waiton", "bad-argv");
    let dir = scratch.path().join("sessions/mac");
    let long_agent = "a".repeat(65);
    let long_session = format!("@{}", "s".repeat(129));
    let mut tails = [
        "",
        "@",
        ":lead",
        "owner:",
        "@owner:lead",
        "../lead",
        "-lead",
        "a:b:c",
        "ö",
        &long_agent,
        &long_session,
    ]
    .iter()
    .map(|target| words(&["waiting-agent", "--on", target, WHY]))
    .collect::<Vec<_>>();
    tails.push(words(&["waiting-agent", "--on"]));
    tails.push(words(&[
        "waiting-agent",
        "--on",
        "lead",
        "--on",
        "colead",
        WHY,
    ]));
    for state in ["working", "waiting-user", "blocked", "done"] {
        tails.push(words(&[state, "--on", "lead", WHY]));
    }
    for tail in tails {
        let out = super::cli::ae()
            .env_remove("TMUX_PANE")
            .arg(ae::cli::STATE)
            .arg(&dir)
            .args(&tail)
            .output()
            .expect("binary runs");
        assert_eq!(out.status.code(), Some(2), "{tail:?}: {:?}", out.stderr);
        assert_eq!(
            std::fs::symlink_metadata(dir.join("events.jsonl"))
                .expect_err("refused argv writes no journal")
                .kind(),
            std::io::ErrorKind::NotFound
        );
    }
}

#[test]
fn waiton_untyped_record_bytes_are_identical_to_the_released_contract() {
    for state in [
        "working",
        "waiting-user",
        "waiting-agent",
        "blocked",
        "done",
    ] {
        let Command::Declare(declaration) = state::parse(&words(&[state, WHY])).expect("old argv")
        else {
            panic!("declaration")
        };
        let mut expected = format!(
            "{{\"ts\":\"2026-10-21T12:00:00Z\",\"actor\":\"lead\",\"action\":\"state\",\"ref\":\"{state}\",\"summary\":\"{WHY}\"}}\n"
        );
        if state == "done" {
            writeln!(expected, "{{\"ts\":\"2026-10-21T12:00:00Z\",\"actor\":\"lead\",\"action\":\"done\",\"summary\":\"{WHY}\"}}")
                .expect("writing a String");
        }
        assert_eq!(state::event_body(now(), "lead", &declaration), expected);
    }
    // Reason prose cannot silently create an edge.
    let reason = format!("{WHY}; literal --on @owner stays prose");
    let Command::Declare(declaration) =
        state::parse(&words(&["waiting-agent", &reason])).expect("reason prose")
    else {
        panic!("declaration")
    };
    let body = state::event_body(now(), "lead", &declaration);
    assert_eq!(
        ae::json::parse(body.trim()).expect("record").get("wait_on"),
        None
    );
}

#[test]
fn waiton_owner_chain_stays_quiet_before_and_after_the_ceiling() {
    for age in [720, 1200, 7200] {
        let (snapshot, runtime) = owner_chain(age);
        let entry = read(&snapshot, &runtime);
        assert_eq!(
            entry.attention, None,
            "no seat asked the human at age {age}"
        );
        assert!(
            !entry.needs_attention(),
            "typed external wait is not human attention"
        );
        assert_eq!(
            entry
                .agents
                .iter()
                .filter(|agent| agent.reason.is_some())
                .count(),
            0
        );
        assert!(
            section(&snapshot, &runtime, &entry).rows.is_empty(),
            "chat Needs you=0"
        );
        let json = entry.to_json();
        assert_eq!(json.get("needs_attention"), Some(&Value::Bool(false)));
        assert_eq!(json.get("wait_on"), None, "no new session JSON key");
        for agent in &entry.agents {
            assert_eq!(
                agent.to_json().get("wait_on"),
                None,
                "no new agent JSON key"
            );
        }
        let (mark, _) = app(&view_world(entry, true));
        assert_eq!(
            mark,
            Mark::WaitingAgent,
            "finished review never finishes session"
        );
    }
}

#[test]
fn waiton_app_seats_and_session_follow_the_literal_owner_chain() {
    let (snapshot, runtime) = owner_chain(720);
    let (_, text) = app(&view_world(read(&snapshot, &runtime), true));
    assert!(
        text.lines()
            .any(|line| line.contains("lead") && line.contains("waiting on colimafix")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line.contains("colimafix") && line.contains("waiting on @owner")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line.contains("colead") && line.contains("finished")),
        "{text}"
    );
    assert!(
        text.contains("waiting on @owner (via colimafix)"),
        "aggregate names the terminal worker: {text}"
    );
    assert!(text.contains("12m"), "own declaration age survives: {text}");
    assert!(
        text.contains("lead: waiting for owner"),
        "own reason survives: {text}"
    );
    assert!(
        !text.contains("not found"),
        "known owner is present: {text}"
    );
}

#[test]
fn waiton_list_seats_and_session_name_the_chain_and_overdue_wait() {
    for age in [720, 7200] {
        let (snapshot, runtime) = owner_chain(age);
        let text = list(&view_world(read(&snapshot, &runtime), true));
        assert!(
            text.lines()
                .any(|line| line.trim_start().starts_with("lead ")
                    && line.contains("waiting on colimafix")),
            "{text}"
        );
        assert!(
            text.lines()
                .any(|line| line.trim_start().starts_with("colimafix ")
                    && line.contains("waiting on @owner")),
            "{text}"
        );
        assert!(text.contains("waiting on @owner (via colimafix)"), "{text}");
        assert!(
            text.contains("lead: waiting for owner"),
            "own reason survives: {text}"
        );
        assert!(
            !text.contains("attn:blocked"),
            "external wait stays quiet: {text}"
        );
        assert_eq!(text.contains("overdue"), age == 7200, "{text}");
        if age == 7200 {
            assert!(text.contains("overdue 2h"), "{text}");
        }
    }
}

#[test]
fn waiton_local_qualified_label_is_preserved_but_traverses_locally() {
    let (snapshot, runtime) = chain(720, r#""mac:colimafix""#, r#""owner:lead""#);
    let entry = read(&snapshot, &runtime);
    let world = view_world(entry, true);
    for text in [list(&world), app(&world).1] {
        assert!(
            text.contains("waiting on mac:colimafix"),
            "exact declared label: {text}"
        );
        assert!(
            text.contains("waiting on owner:lead"),
            "remote agent label: {text}"
        );
        assert!(
            text.contains("waiting on @owner (via colimafix)"),
            "local edge followed: {text}"
        );
    }
}

#[test]
fn waiton_cycle_untyped_and_mixed_external_destinations_stay_in_session() {
    for (lead, worker) in [
        (Some("colimafix"), Some("lead")),
        (None, Some("@owner")),
        (Some("@other"), Some("@owner")),
        (Some("@mac"), Some("@mac")),
    ] {
        let extra =
            |on: Option<&str>| on.map_or_else(String::new, |on| format!(r#","wait_on":"{on}""#));
        let (snapshot, runtime) = fixture(vec![
            event("main", "waiting-agent", 720, &extra(lead)),
            event("worker.0", "done", 720, ""),
            event("spawned.0", "waiting-agent", 720, &extra(worker)),
        ]);
        let world = view_world(read(&snapshot, &runtime), true);
        for text in [list(&world), app(&world).1] {
            assert!(
                text.contains("waiting (in session)"),
                "no invented external aggregate: {text}"
            );
        }
    }
}

#[test]
fn waiton_working_terminal_does_not_read_as_waiting_or_finished() {
    let (snapshot, runtime) = fixture(vec![
        event("main", "waiting-agent", 720, r#","wait_on":"colimafix""#),
        event("worker.0", "done", 720, ""),
        event("spawned.0", "working", 0, ""),
    ]);
    let world = view_world(read(&snapshot, &runtime), false);
    let (mark, text) = app(&world);
    assert_eq!(mark, Mark::Working, "chain ends at active work");
    assert!(!text.contains("waiting (in session)"), "{text}");
    assert!(!text.contains("waiting on @"), "{text}");
    assert!(
        list(&world)
            .lines()
            .any(|line| line.contains("mac") && line.contains("working")),
        "session working words"
    );
}

#[test]
fn waiton_missing_target_is_named_without_guessing_and_remains_external() {
    let (snapshot, runtime) = owner_chain(7200);
    let entry = read(&snapshot, &runtime);
    assert!(
        !entry.needs_attention(),
        "missing external is still a typed wait"
    );
    let world = view_world(entry, false);
    for text in [list(&world), app(&world).1] {
        assert!(
            text.contains("@owner (not found)"),
            "snapshot proves missing owner: {text}"
        );
    }
    let (snapshot, runtime) = chain(720, r#""ghost""#, r#""@owner""#);
    let world = view_world(read(&snapshot, &runtime), true);
    for text in [list(&world), app(&world).1] {
        assert!(
            text.contains("ghost (not found)"),
            "missing seat stays declared: {text}"
        );
    }
}

#[test]
fn waiton_malformed_field_never_discards_the_valid_untyped_state() {
    for raw in [
        "null",
        "true",
        "7",
        "{}",
        "[]",
        r#""""#,
        r#""@""#,
        r#""a:b:c""#,
        r#""@owner","wait_on":"@owner""#,
        r#""@owner","wait_on":7"#,
        r#"7,"wait_on":"@owner""#,
    ] {
        let (snapshot, runtime) = fixture(vec![event(
            "main",
            "waiting-agent",
            1200,
            &format!(r#","wait_on":{raw},"future_wait_fact":{{}}"#),
        )]);
        let entry = read(&snapshot, &runtime);
        assert_eq!(
            entry.agents[0].state.as_deref(),
            Some("waiting-agent"),
            "{raw}"
        );
        assert_eq!(
            entry.agents[0].reason,
            Some(Reason::Blocked),
            "untyped ceiling still applies: {raw}"
        );
        assert!(
            !list(&view_world(entry, false)).contains("waiting on @owner"),
            "{raw}"
        );
    }
}

#[test]
fn waiton_independent_human_attention_and_untyped_escalation_stay_intact() {
    for (state, alert, expected) in [
        ("blocked", None, Reason::Blocked),
        ("waiting-user", None, Reason::WaitingUser),
        ("waiting-agent", Some(Reason::Limit), Reason::Limit),
        ("waiting-agent", Some(Reason::Dead), Reason::Dead),
        ("waiting-agent", Some(Reason::Throttled), Reason::Throttled),
    ] {
        let (mut snapshot, mut runtime) = owner_chain(7200);
        if state != "waiting-agent" {
            let (replacement, _) =
                fixture(vec![event("main", state, 7200, r#","wait_on":"@owner""#)]);
            snapshot.events.as_mut().expect("read").events[0] =
                replacement.events.expect("read").events[0].clone();
        }
        runtime.agents[0].alert = alert;
        let entry = read(&snapshot, &runtime);
        assert_eq!(entry.agents[0].reason, Some(expected));
        assert!(
            entry.needs_attention(),
            "real independent attention survives"
        );
        assert!(!section(&snapshot, &runtime, &entry).rows.is_empty());
    }
    let (snapshot, runtime) = fixture(vec![event("main", "waiting-agent", 1200, "")]);
    assert_eq!(
        read(&snapshot, &runtime).agents[0].reason,
        Some(Reason::Blocked)
    );
    let (snapshot, runtime) = chain(1200, r#""colimafix""#, r#""lead""#);
    assert_eq!(
        read(&snapshot, &runtime).agents[0].reason,
        Some(Reason::Blocked)
    );
    let (snapshot, runtime) = chain(1200, r#""@mac""#, r#""@mac""#);
    assert_eq!(
        read(&snapshot, &runtime).agents[0].reason,
        Some(Reason::Blocked),
        "a wait on this session receives no external exemption"
    );
}

#[test]
fn waiton_session_is_finished_only_when_every_live_seat_declared_done() {
    let (snapshot, runtime) = fixture(
        SEATS
            .iter()
            .map(|(slot, _)| event(slot, "done", 10, ""))
            .collect(),
    );
    let (mark, _) = app(&view_world(read(&snapshot, &runtime), false));
    assert_eq!(mark, Mark::Done);
    let (snapshot, runtime) = owner_chain(720);
    let (mark, _) = app(&view_world(read(&snapshot, &runtime), true));
    assert_eq!(
        mark,
        Mark::WaitingAgent,
        "one finished review does not finish rollout"
    );
}

#[test]
fn waiton_app_external_wait_draws_overdue_only_past_the_ceiling() {
    for age in [720, 7200] {
        let (snapshot, runtime) = owner_chain(age);
        let (mark, text) = app(&view_world(read(&snapshot, &runtime), true));
        assert_eq!(mark, Mark::WaitingAgent);
        assert_eq!(text.contains("overdue"), age == 7200, "{text}");
        if age == 7200 {
            assert!(text.contains("overdue 2h"), "{text}");
        }
    }
}

#[test]
fn waiton_working_aggregate_is_named_on_the_list_session_row() {
    let (snapshot, runtime) = fixture(vec![
        event("main", "waiting-agent", 720, r#","wait_on":"colimafix""#),
        event("worker.0", "done", 720, ""),
        event("spawned.0", "working", 0, ""),
    ]);
    let text = list(&view_world(read(&snapshot, &runtime), false));
    assert!(
        text.lines()
            .any(|line| line.starts_with("mac ") && line.contains("working")),
        "only the session header supplies this claim: {text}"
    );
}

mod source_seam {
    //! Frozen pure graph and source-gate pins. Oracle: contract-r2 + lead @current ruling.

    #![allow(clippy::expect_used, reason = "literal target fixtures must parse")]

    use ae::harness_state::HarnessState;
    use ae::session::OwnWork;
    use ae::wait_on::{End, Kind, Link, Target, end_from, ends_elsewhere, session_end};
    use ae::watchdog::{DoneProgress, QuietKind, WaitProgress, WaitState};
    use ae::watchdog_daemon::{
        Effect, HarnessObservation, Knobs, Observation, PaneState, Verdict, account,
    };

    fn target(raw: &str) -> Target {
        Target::parse(raw, "mac").expect("grammar-valid fixture")
    }

    fn link<'a>(name: &'a str, on: Option<&'a Target>, waiting: bool, working: bool) -> Link<'a> {
        Link {
            name,
            on,
            waiting,
            working,
        }
    }

    #[test]
    fn waiton_seam_grammar_uses_existing_names_and_keeps_declared_spelling() {
        for (raw, expected) in [
            ("colimafix", Kind::Seat("colimafix".to_owned())),
            ("mac:colimafix", Kind::Seat("colimafix".to_owned())),
            (
                "owner:lead",
                Kind::Remote {
                    session: "owner".to_owned(),
                    agent: "lead".to_owned(),
                },
            ),
            ("@owner", Kind::Session("owner".to_owned())),
            ("@mac", Kind::Session("mac".to_owned())),
        ] {
            let got = target(raw);
            assert_eq!(got.label(), raw);
            assert_eq!(got.kind(), &expected);
        }
        let agent_max = "a".repeat(64);
        let session_max = format!("@{}", "s".repeat(128));
        for raw in [&agent_max, &session_max] {
            assert_eq!(target(raw).label(), raw);
        }
        for raw in [
            "", "@", ":a", "s:", "@s:a", "a:b:c", "../a", "-a", "ö", "a\nb", "a b",
        ] {
            assert!(Target::parse(raw, "mac").is_none(), "{raw:?}");
        }
        assert!(Target::parse(&"a".repeat(65), "mac").is_none());
        assert!(Target::parse(&format!("@{}", "s".repeat(129)), "mac").is_none());
    }

    #[test]
    fn waiton_seam_owner_chain_and_qualified_local_share_one_external_terminal() {
        for declared in ["colimafix", "mac:colimafix"] {
            let local = target(declared);
            for remote in ["@owner", "owner:lead"] {
                let remote = target(remote);
                let links = [
                    link("lead", Some(&local), true, false),
                    link("colimafix", Some(&remote), true, false),
                    link("colead", None, false, false),
                ];
                let expected = Some(End::External {
                    session: "owner".to_owned(),
                    via: "colimafix".to_owned(),
                });
                assert_eq!(end_from(&links, "lead"), expected);
                assert_eq!(session_end(&links), expected);
                assert_eq!(end_from(&links, "colead"), None);
                assert!(ends_elsewhere(&links, "lead"));
                assert!(ends_elsewhere(&links, "colimafix"));
                assert_eq!(
                    session_end(&links).expect("waiting chain").words(),
                    "waiting on @owner (via colimafix)"
                );
            }
        }
    }

    #[test]
    fn waiton_seam_self_session_is_accepted_but_never_external_or_exempt() {
        let local = target("colimafix");
        let same = target("@mac");
        let links = [
            link("lead", Some(&local), true, false),
            link("colimafix", Some(&same), true, false),
        ];
        assert_eq!(same.label(), "@mac");
        for seat in ["lead", "colimafix"] {
            assert_eq!(end_from(&links, seat), Some(End::InSession));
            assert!(!ends_elsewhere(&links, seat));
        }
        assert_eq!(session_end(&links), Some(End::InSession));
    }

    #[test]
    fn waiton_seam_cycles_missing_local_untyped_and_noncurrent_edges_stop() {
        let to_worker = target("colimafix");
        let back = target("lead");
        let remote = target("@owner");
        for links in [
            vec![
                link("lead", Some(&to_worker), true, false),
                link("colimafix", Some(&back), true, false),
            ],
            vec![link("lead", Some(&back), true, false)],
            vec![link("lead", Some(&to_worker), true, false)],
            vec![
                link("lead", None, true, false),
                link("colimafix", Some(&remote), true, false),
            ],
            // A stale target on an endpoint that is no longer currently waiting cannot be followed.
            vec![
                link("lead", Some(&to_worker), true, false),
                link("colimafix", Some(&remote), false, false),
            ],
        ] {
            assert_eq!(end_from(&links, "lead"), Some(End::InSession));
            assert!(!ends_elsewhere(&links, "lead"));
            assert_eq!(session_end(&links), Some(End::InSession));
        }
        assert_eq!(session_end(&[]), None);
    }

    #[test]
    fn waiton_seam_working_endpoint_wins_and_mixed_external_destinations_fall_back() {
        let local = target("colimafix");
        let owner = target("@owner");
        let other = target("@other");
        let working = [
            link("lead", Some(&local), true, false),
            link("colimafix", Some(&owner), false, true),
        ];
        assert_eq!(end_from(&working, "lead"), Some(End::Working));
        assert_eq!(session_end(&working), Some(End::Working));
        assert_eq!(End::Working.words(), "working");
        assert!(!ends_elsewhere(&working, "lead"));
        let mixed = [
            link("lead", Some(&other), true, false),
            link("colimafix", Some(&owner), true, false),
        ];
        assert_eq!(session_end(&mixed), Some(End::InSession));
        assert_eq!(End::InSession.words(), "waiting (in session)");
        // Per-seat source exemption still uses that seat's chain, not the session's fallback.
        assert!(ends_elsewhere(&mixed, "lead"));
        assert!(ends_elsewhere(&mixed, "colimafix"));
    }

    #[test]
    fn waiton_seam_one_destination_with_different_vias_names_only_the_session() {
        let owner = target("@owner");
        let links = [
            link("lead", Some(&owner), true, false),
            link("colimafix", Some(&owner), true, false),
        ];
        assert_eq!(
            session_end(&links).expect("both seats wait").words(),
            "waiting on @owner",
            "same destination remains external without inventing a via"
        );
        assert!(ends_elsewhere(&links, "lead"));
        assert!(ends_elsewhere(&links, "colimafix"));
    }

    fn observation(external: bool, progress: WaitProgress) -> Observation {
        Observation {
            now_epoch: 1_800_000_000,
            hash: 7,
            harness: HarnessObservation {
                frame: HarnessState::Idle,
                human_draft: false,
                durable_stale: false,
                declaration: Some(1),
            },
            identity: 1,
            is_dead: false,
            throttle: None,
            limit_notice: None,
            capture_ok: true,
            human_prompt: None,
            throttle_quota: None,
            quiet: Some(QuietKind::WaitingAgent),
            ended_wait: None,
            done_progress: DoneProgress::None,
            wait_progress: progress,
            wait_ends_elsewhere: external,
            target_moved: None,
            descendancy: ae::procs::Descendancy::Present,
            launch_attempt: ae::tmux::Evidence::Silent,
            outstanding_alert: None,
            unreachable: false,
            failed_elsewhere: false,
            prompt_named: false,
            start: None,
            last_actor_event_age_secs: 7200,
            declared_age_secs: 7200,
            sweep: None,
            own_work: OwnWork::default(),
            auto_in_flight: None,
            auto_deadline: None,
        }
    }

    #[test]
    fn waiton_seam_daemon_skips_ceiling_booking_at_source_and_keeps_challenge() {
        let remote = target("@owner");
        let links = [link("lead", Some(&remote), true, false)];
        let external = ends_elsewhere(&links, "lead");
        for cadence in [0, 300] {
            let knobs = Knobs {
                idle_nudge_secs: cadence,
                ..Knobs::default()
            };
            assert!(7200 > ae::watchdog::waiting_agent_cap_secs(cadence));
            let seen = observation(external, WaitProgress::None);
            let got = account(&PaneState::default(), &seen, &knobs);
            assert_eq!(got.verdict, Verdict::Quiet(QuietKind::WaitingAgent));
            assert!(!got.effects.iter().any(|effect| matches!(
                effect,
                Effect::Nudge
                    | Effect::Emit {
                        action: "alert",
                        ..
                    }
            )));
            let local = account(
                &PaneState::default(),
                &observation(false, WaitProgress::None),
                &knobs,
            );
            assert_eq!(local.verdict, Verdict::Quiet(QuietKind::Blocked));
            assert_eq!(local.effects.contains(&Effect::Nudge), cadence > 0);
        }
        let due = WaitProgress::ChallengeDue {
            confirmations: 0,
            required: 2,
            wait_age_secs: 7200,
            attempts: 0,
        };
        for (external, escalated) in [(true, false), (false, true)] {
            let got = account(
                &PaneState::default(),
                &observation(external, due),
                &Knobs::default(),
            );
            assert!(got.effects.contains(&Effect::WaitChallenge {
                confirmations: 0,
                required: 2,
                wait_age_secs: 7200,
                state: WaitState::WaitingAgent,
                escalated
            }));
            assert!(
                !got.effects.contains(&Effect::Nudge),
                "challenge owns delivery"
            );
        }
    }

    #[test]
    fn waiton_seam_external_wait_still_obeys_lapsed_challenge_nudge_lifecycle() {
        let seen = observation(
            true,
            WaitProgress::Lapsed {
                confirmations: 0,
                required: 2,
            },
        );
        let got = account(&PaneState::default(), &seen, &Knobs::default());
        assert_eq!(got.verdict, Verdict::Quiet(QuietKind::WaitingAgent));
        assert!(
            got.effects.contains(&Effect::Nudge),
            "ordinary lapsed-wait nudges remain"
        );
        assert!(
            !got.effects
                .iter()
                .any(|effect| matches!(effect, Effect::WaitChallenge { .. }))
        );
    }
}
// Contract 4: absence is a fact of a COMPLETE roster, including a remote
// session whose roster the already-loaded world owns. Unknown is not absent.
fn waiton_target_roster(session: &str, complete: bool) -> Meta {
    let damage = if complete {
        ""
    } else {
        "this line has no equals sign\n"
    };
    Meta::parse(&format!(
        "schema=2\nmode=local\nsession={session}\nsession_id={UUID}\nidle_nudge_secs=300\ndone_confirmations=2\nseat.main=lead\nprofile.main=demo\nagent_bin.main=codex\n{damage}"
    ))
}

#[test]
fn waiton_local_target_is_missing_only_when_its_roster_is_complete() {
    for complete in [true, false] {
        let (mut snapshot, runtime) = fixture(vec![event(
            "main",
            "waiting-agent",
            720,
            r#","wait_on":"ghost""#,
        )]);
        snapshot.meta = Some(waiton_target_roster("mac", complete));
        let entry = read(&snapshot, &runtime);
        assert_eq!(entry.degraded, !complete, "roster provenance fixture");
        let world = view_world(entry, true);
        for text in [list(&world), app(&world).1] {
            assert!(text.contains("waiting on ghost"), "declared label: {text}");
            assert_eq!(
                text.contains("ghost (not found)"),
                complete,
                "local absence requires a complete roster: {text}"
            );
        }
    }
}

#[test]
fn waiton_remote_agent_absence_uses_its_loaded_complete_roster() {
    for complete in [true, false] {
        let (snapshot, runtime) = chain(7200, r#""colimafix""#, r#""owner:ghost""#);
        let entry = read(&snapshot, &runtime);
        assert!(!entry.needs_attention(), "missing external remains exempt");
        let (mut remote, remote_runtime) = fixture(Vec::new());
        remote.meta = Some(waiton_target_roster("owner", complete));
        let owner = ae::session::entry_from(&remote, "owner", &remote_runtime, now(), 1800);
        assert_eq!(
            owner.degraded, !complete,
            "remote roster provenance fixture"
        );
        let world = World::new(now(), vec![entry, owner]);
        for text in [list(&world), app(&world).1] {
            assert!(
                text.contains("waiting on owner:ghost"),
                "declared label: {text}"
            );
            assert_eq!(
                text.contains("owner:ghost (not found)"),
                complete,
                "loaded remote roster proves absence only when complete: {text}"
            );
        }
    }
}

#[test]
fn waiton_incomplete_fleet_never_proves_an_external_session_missing() {
    use ae::inventory::{FailedSource, ServerId};
    use ae::meta::Selector;

    for loss in [
        FailedSource::CanonicalRoot("/unreadable-sessions".into()),
        FailedSource::Server(ServerId::Selected(Selector::Name("unreadable".to_owned()))),
    ] {
        let (snapshot, runtime) = owner_chain(7200);
        let entry = read(&snapshot, &runtime);
        assert!(!entry.needs_attention(), "external overdue remains exempt");
        let world = view_world(entry, false).with_losses(&[loss]);
        assert!(!world.inventory_complete(), "lost-source fixture");
        for text in [list(&world), app(&world).1] {
            assert!(text.contains("waiting on @owner"), "declared label: {text}");
            assert!(
                !text.contains("not found"),
                "incomplete fleet proves no session absent: {text}"
            );
        }
    }
}
