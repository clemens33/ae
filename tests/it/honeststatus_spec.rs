//! Frozen acceptance for honeststatus A+B+C.
//! Oracle: honeststatus brief + delta r2 and the human ruling that session
//! status means whether the human is needed. Current code supplies interfaces,
//! never expected results. Only driver installs these bytes; revisions are new files.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance uses synthetic journals and owned private fixtures"
)]

use std::collections::BTreeMap;

use ae::app::{fleet, overview};
use ae::attention::Reason;
use ae::console::needs::{self, Inputs, Verdict};
use ae::digest::{AgentEntry, SessionEntry, Status};
use ae::events::{Cursor, Drain, Event, SessionKey};
use ae::filters::ListArgs;
use ae::inventory::MetaRead;
use ae::json::Value;
use ae::listing::World;
use ae::meta::Meta;
use ae::session::{AgentRuntime, RecordSnapshot, SessionRead, SessionRuntime};
use ae::theme::{FleetOrder, Mark};
use ae::time::Timestamp;
use ae::tmux::{Evidence, PickerAgent};

const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const OTHER_UUID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const NOW: i64 = 1_792_483_200;
const LIMIT: i64 = 1800;

fn now() -> Timestamp {
    Timestamp::from_epoch(NOW)
}

// This table is the requirement, independent of either the rank or glyph table.
const REASONS: [(Reason, bool); 6] = [
    (Reason::Stale, false),
    (Reason::Throttled, true),
    (Reason::Limit, true),
    (Reason::Blocked, true),
    (Reason::WaitingUser, true),
    (Reason::Dead, true),
];

fn agent(reason: Option<Reason>) -> AgentEntry {
    AgentEntry {
        name: "lead".to_owned(),
        reference: "lead".to_owned(),
        alive: Some(true),
        state: Some("working".to_owned()),
        reason,
        ..AgentEntry::default()
    }
}

fn entry(reason: Option<Reason>) -> SessionEntry {
    let mut entry = SessionEntry::new("one", Status::Running);
    entry.attention = reason;
    entry.agents.push(agent(reason));
    entry.last_active_epoch = Some(NOW);
    entry
}

fn seats() -> BTreeMap<String, fleet::Facts> {
    BTreeMap::from([(
        "one".to_owned(),
        fleet::Facts::Seats {
            id: "$1".to_owned(),
            agents: vec![PickerAgent {
                name: "lead".to_owned(),
                profile: "idle".to_owned(),
                state: "working".to_owned(),
                pane: "%1".to_owned(),
                client: String::new(),
                model: String::new(),
                effort: String::new(),
                drift: false,
            }],
        },
    )])
}

#[test]
fn honeststatus_published_mark_rank_meanings_remain_compatible() {
    for (rank, expected) in [
        (0, Mark::Idle),
        (1, Mark::Done),
        (2, Mark::Working),
        (3, Mark::Stale),
        (4, Mark::NeedsYou),
        (5, Mark::Dead),
    ] {
        assert_eq!(
            Mark::from_rank_value(rank),
            expected,
            "published rank {rank}"
        );
        assert_eq!(expected.rank(), rank);
    }
    assert_eq!(Mark::WaitingAgent.rank(), 2);
}

#[test]
fn honeststatus_json_list_filter_and_next_agree_on_actual_human_needs() {
    let args = ListArgs::parse(&["--needs-attn".to_owned()]).expect("public list flag");
    for (reason, human) in REASONS {
        let entry = entry(Some(reason));
        assert_eq!(entry.needs_attention(), human, "{reason}");
        assert_eq!(
            entry.to_json().get("needs_attention"),
            Some(&Value::Bool(human)),
            "{reason}"
        );
        let world = World::new(now(), vec![entry]);
        assert_eq!(
            args.selection.select(&world.sessions, now()).len(),
            usize::from(human),
            "{reason}"
        );
        assert_eq!(
            ae::next::choose(&world).is_some(),
            human,
            "next must not send the human to {reason}"
        );
    }
}

#[test]
fn honeststatus_monitor_does_not_announce_unanswered_or_stale_as_needs_you() {
    for (reason, human) in REASONS {
        let world = World::new(now(), vec![entry(Some(reason))]);
        let observed = ae::monitor::observe(&world, NOW, 1200);
        assert_eq!(observed.len(), 1);
        let args = ae::monitor::Args {
            now: NOW,
            quiet_secs: 1200,
            liveness_sweeps: 100,
            init: false,
            dry_run: true,
            notify: false,
            format: ae::monitor::Format::Text,
        };
        let text = ae::monitor::sweep(None, &observed, &args).text();
        assert_eq!(
            text.contains("needs you:"),
            human,
            "monitor: {reason}: {text}"
        );
        assert!(text.contains(reason.as_str()), "fact stays visible: {text}");
        if !human {
            assert!(
                !text.contains('⚠'),
                "quiet stale is not a human alert: {text}"
            );
        }
    }
}

fn snapshot(read: SessionRead) -> RecordSnapshot {
    RecordSnapshot {
        meta: Some(Meta::parse(&format!(
            "schema=2\nsession_id={UUID}\nlayout=lead-pair\nseat.main=lead\nprofile.main=idle\n"
        ))),
        meta_read: MetaRead::Parsed,
        events: Some(read),
        legacy_created_epoch: None,
    }
}

fn runtime() -> SessionRuntime {
    SessionRuntime {
        status: Status::Running,
        branch: Some("fixture".to_owned()),
        agents: vec![AgentRuntime {
            pane_id: None,
            slot: "main".to_owned(),
            alive: Some(true),
            alert: None,
            observed: ae::harness_state::HarnessState::Unknown,
        }],
    }
}

fn empty_read() -> SessionRead {
    read(Vec::new())
}

fn read(events: Vec<Event>) -> SessionRead {
    SessionRead::from_drain_in(
        &Drain {
            events,
            cursor: Cursor::default(),
            skipped: Vec::new(),
            drained: true,
        },
        SessionKey::new("one", UUID),
    )
}

#[test]
fn honeststatus_console_fleet_and_overview_use_the_same_human_needs_table() {
    let snapshot = snapshot(empty_read());
    let runtime = runtime();
    for (reason, human) in REASONS {
        let entry = entry(Some(reason));
        let folded = needs::fold(&Inputs {
            session: "one",
            snapshot: &snapshot,
            entry: &entry,
            runtime: &runtime,
            runtime_read: true,
            beat: Evidence::At(NOW),
            lead_pair: true,
            now: now(),
        })
        .expect("fully verified synthetic read");
        assert_eq!(
            folded.rows.len(),
            usize::from(human),
            "chat needs-you: {reason}"
        );
        let world = World::new(now(), vec![entry.clone()]);
        let needs = BTreeMap::from([("one".to_owned(), folded.clone())]);
        let fleet = fleet::rows(
            &world,
            &seats(),
            &BTreeMap::new(),
            &FleetOrder::EMPTY,
            &needs,
            Some("one"),
            now(),
        );
        assert_eq!(fleet.rows[0].needy, human, "app needs-you count: {reason}");
        assert_eq!(
            overview::of(&entry, Some(&folded), None, now()).open.len(),
            usize::from(human),
            "app open questions: {reason}"
        );
        if !human {
            assert_ne!(fleet.rows[0].mark, Mark::NeedsYou, "app fleet: {reason}");
        }
    }
}

fn event(epoch: i64, actor: &str, action: &str, target: &str, id: &str, extra: &str) -> Event {
    let stamp = Timestamp::from_epoch(epoch).to_string();
    Event::parse_line(&format!(
        "{{\"ts\":\"{stamp}\",\"actor\":\"{actor}\",\"action\":\"{action}\",\"target\":\"{target}\",\"ref\":\"{id}\",\"summary\":\"synthetic work\"{extra}}}"
    )).expect("synthetic ledger record")
}

fn ask(actor: &str, age: i64) -> Event {
    let route = if actor == "lead" {
        format!(
            ",\"actor_slot\":\"main\",\"actor_session\":\"one\",\"actor_session_id\":\"{UUID}\""
        )
    } else {
        String::new()
    };
    event(
        NOW - age,
        actor,
        "ask",
        "helper",
        "ae-20261020T080000Z-aaaaaaaa",
        &format!(
            "{route},\"target_slot\":\"spawned.0\",\"target_session\":\"one\",\"target_session_id\":\"{UUID}\""
        ),
    )
}

#[test]
fn honeststatus_human_work_orders_and_agent_requests_remain_visible_but_quiet() {
    for actor in ["lead", "console:local", "telegram:42", "discord:42"] {
        for (age, count) in [(LIMIT, 0), (LIMIT + 1, 1)] {
            let read = read(vec![ask(actor, age)]);
            assert_eq!(read.pending.len(), 1, "open work never disappears: {actor}");
            assert_eq!(
                read.unanswered(now(), LIMIT).len(),
                count,
                "strict threshold: {actor}"
            );
            let entry = ae::session::entry_from(&snapshot(read), "one", &runtime(), now(), LIMIT);
            assert!(
                !entry.needs_attention(),
                "{actor} has not asked the human to intervene"
            );
            assert_eq!(
                entry.attention, None,
                "request debt is separate from seat attention"
            );
            assert_eq!(
                entry.to_json().get("unanswered"),
                Some(&Value::Num(
                    i64::try_from(count).expect("bounded fixture count")
                ))
            );
            let listed = ae::listing::table_at(&[&entry], now());
            assert_eq!(
                listed.contains("unanswered:1"),
                count == 1,
                "quiet named debt: {listed}"
            );
            assert!(!listed.contains("attn:unanswered"));
        }
    }
}

#[test]
fn honeststatus_real_human_wait_outranks_quiet_request_debt() {
    let wait = event(
        NOW - 60,
        "lead",
        "state",
        "",
        "waiting-user",
        ",\"actor_slot\":\"main\",\"actor_session\":\"one\"",
    );
    let read = read(vec![ask("console:local", LIMIT + 1), wait]);
    let snapshot = snapshot(read);
    let runtime = runtime();
    let entry = ae::session::entry_from(&snapshot, "one", &runtime, now(), LIMIT);
    assert_eq!(entry.attention, Some(Reason::WaitingUser));
    assert!(entry.needs_attention());
    let needs = needs::fold(&Inputs {
        session: "one",
        snapshot: &snapshot,
        entry: &entry,
        runtime: &runtime,
        runtime_read: true,
        beat: Evidence::At(NOW),
        lead_pair: true,
        now: now(),
    })
    .expect("fully verified mixed facts");
    assert_eq!(needs.rows.len(), 1);
    assert_eq!(needs.rows[0].verdict, Verdict::Reason(Reason::WaitingUser));
    assert_eq!(
        snapshot.events.as_ref().expect("journal").pending.len(),
        1,
        "human wait must not close the work order"
    );
}

#[test]
fn honeststatus_brief_names_unanswered_outside_its_needs_you_section() {
    let rig = RequestRig::new("brief", "ask", false);
    for (age, count) in [(LIMIT, 0), (LIMIT + 1, 1)] {
        let entry = ae::session::entry_from(
            &snapshot(read(vec![ask("console:local", age)])),
            "one",
            &runtime(),
            now(),
            LIMIT,
        );
        let card = ae::brief::card_for(&entry, &rig.dir("one"), None, false, now(), None);
        assert_eq!(card.human_needs().count(), 0);
        let text = ae::brief::render(&[card]);
        assert_eq!(
            text.contains("unanswered:1"),
            count == 1,
            "same strict threshold: {text}"
        );
        let human_section = text.split_once("needs you:").expect("human section").1;
        assert!(
            human_section
                .lines()
                .next()
                .expect("value")
                .contains("none recorded"),
            "{text}"
        );
    }
}

#[test]
fn honeststatus_brief_claims_every_actual_human_reason_for_main_and_spawned() {
    let rig = RequestRig::new("briefneeds", "ask", false);
    rig.write_log("");
    for (reason, human) in REASONS {
        for (name, reference) in [("lead", "lead"), ("helper", "helper")] {
            let mut entry = entry(Some(reason));
            entry.main_agent = Some("lead".to_owned());
            entry.agents[0].name = name.to_owned();
            entry.agents[0].reference = reference.to_owned();
            let card = ae::brief::card_for(&entry, &rig.dir("one"), None, false, now(), None);
            assert_eq!(
                card.human_needs().count(),
                usize::from(human),
                "{name}: {reason}"
            );
            assert_eq!(card.urgency() > 0, human, "brief ordering: {reason}");
        }
    }
}

const REQ: &str = "ae-20261020T080000Z-aaaaaaaa";

struct RequestRig {
    scratch: super::cli::OwnedScratch,
    socket: std::path::PathBuf,
    asker: String,
    target: String,
    target_session: &'static str,
}

impl RequestRig {
    fn new(tag: &str, kind: &str, cross: bool) -> Self {
        let mut scratch = super::cli::OwnedScratch::root("hs", tag);
        let socket = scratch.path().join("sock");
        scratch.add_tmux_server(socket.clone());
        let mut rig = Self {
            scratch,
            socket,
            asker: String::new(),
            target: String::new(),
            target_session: if cross { "two" } else { "one" },
        };
        rig.asker = rig.start("one", "main", "lead", UUID);
        rig.target = if cross {
            rig.start("two", "spawned.0", "helper", OTHER_UUID)
        } else {
            let pane = rig.tmux(&["split-window", "-d", "-t", "one", "-P", "-F", "#{pane_id}"]);
            rig.stamp(pane.trim(), "spawned.0", "helper");
            pane.trim().to_owned()
        };
        for (session, uuid) in [
            ("one", UUID),
            (rig.target_session, if cross { OTHER_UUID } else { UUID }),
        ] {
            let dir = rig.dir(session);
            std::fs::create_dir_all(&dir).expect("isolated session dir");
            std::fs::write(dir.join("meta"), format!(
                "schema=2\nsession={session}\nsession_id={uuid}\nlayout=lead-pair\nmode=local\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nprofile.main=idle\nseat.spawned.0=helper\nprofile.spawned.0=idle\n",
                rig.socket.display()
            )).expect("synthetic metadata");
            for name in ["withdraw", "accept", "requests", "reply"] {
                let path = dir.join(name);
                if !path.exists() {
                    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), path)
                        .expect("helper symlink");
                }
            }
        }
        let target_id = if cross { OTHER_UUID } else { UUID };
        let stamp = Timestamp::from_epoch(NOW - 7200).to_string();
        let log = format!(
            "{{\"ts\":\"{stamp}\",\"actor\":\"lead\",\"action\":\"{kind}\",\"target\":\"helper\",\"ref\":\"{REQ}\",\"summary\":\"synthetic work order\",\"actor_slot\":\"main\",\"actor_session\":\"one\",\"actor_session_id\":\"{UUID}\",\"target_slot\":\"spawned.0\",\"target_session\":\"{}\",\"target_session_id\":\"{target_id}\",\"target_server\":\"{}\",\"target_pane\":\"{}\",\"target_session_uuid\":\"{target_id}\",\"caller_server\":\"{}\",\"caller_pane\":\"{}\",\"caller_session_uuid\":\"{UUID}\"}}\n",
            rig.target_session,
            rig.socket.display(),
            rig.target,
            rig.socket.display(),
            rig.asker,
        );
        rig.write_log(&log);
        rig
    }

    fn dir(&self, session: &str) -> std::path::PathBuf {
        self.scratch.path().join("sessions").join(session)
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|word| (*word).to_owned()));
        let (ok, text) = super::phase2::run_tmux(&args, self.scratch.path());
        assert!(ok, "private tmux {tail:?}: {text}");
        text
    }

    fn start(&self, session: &str, slot: &str, name: &str, uuid: &str) -> String {
        self.tmux(&["-f", "/dev/null", "new-session", "-d", "-s", session]);
        self.tmux(&["set-option", "-t", session, "@ae_session_uuid", uuid]);
        let pane = self
            .tmux(&["display-message", "-p", "-t", session, "#{pane_id}"])
            .trim()
            .to_owned();
        self.stamp(&pane, slot, name);
        pane
    }

    fn stamp(&self, pane: &str, slot: &str, name: &str) {
        self.tmux(&["set-option", "-p", "-t", pane, "@ae_slot", slot]);
        self.tmux(&["set-option", "-p", "-t", pane, "@ae_agent", name]);
    }

    fn write_log(&self, text: &str) {
        for session in ["one", self.target_session] {
            std::fs::write(self.dir(session).join("events.jsonl"), text)
                .expect("synthetic journal");
        }
    }

    fn log(&self, session: &str) -> String {
        std::fs::read_to_string(self.dir(session).join("events.jsonl")).expect("read owned journal")
    }

    fn run(
        &self,
        session: &str,
        pane: &str,
        short: bool,
        helper: &str,
        tail: &[&str],
    ) -> (i32, String, String) {
        let mut run = if short {
            super::cli::ae()
        } else {
            super::cli::helper(&self.dir(session).join(helper))
        };
        if short {
            run.args([&format!("@{session}"), helper]);
        }
        let output = run
            .args(tail)
            .env("AE_HOME", self.scratch.path())
            .env("CONFIG_FILE", self.scratch.path().join("config"))
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("TMUX_PANE", pane)
            .env("TMUX", format!("{},0,0", self.socket.display()))
            .output()
            .expect("helper invocation");
        (
            output.status.code().expect("helper exit code"),
            String::from_utf8(output.stdout).expect("UTF-8 stdout"),
            String::from_utf8(output.stderr).expect("UTF-8 stderr"),
        )
    }

    fn invoke(
        &self,
        session: &str,
        pane: &str,
        short: bool,
        helper: &str,
        id: &str,
        body: &str,
    ) -> (i32, String, String) {
        self.run(session, pane, short, helper, &[id, body])
    }
}

fn status(log: &str) -> ae::requests::Status {
    let states = ae::requests::states(log.as_bytes());
    assert_eq!(states.len(), 1, "one opening remains one ledger row");
    states[0].status
}

#[test]
fn honeststatus_withdraw_closes_own_ask_and_review_on_both_helper_spellings() {
    for (tag, kind, short) in [("wa", "ask", false), ("wr", "review", true)] {
        let rig = RequestRig::new(tag, kind, false);
        let out = rig.invoke(
            "one",
            &rig.asker,
            short,
            "withdraw",
            REQ,
            "resolved independently",
        );
        assert_eq!(out.0, 0, "withdraw own {kind}: {out:?}");
        let log = rig.log("one");
        assert_eq!(status(&log), ae::requests::Status::Cancelled);
        let read = SessionRead::open(&rig.dir("one")).expect("withdrawn journal read");
        assert!(read.pending.is_empty());
        assert!(read.unanswered(now(), LIMIT).is_empty());
        let work = ae::session::Outstanding::read_in(
            &read.events,
            SessionKey::new("one", UUID),
            &["lead".to_owned()],
        );
        assert_eq!(
            work.of(ae::session::Seat::new("one", "main", "lead"))
                .requests,
            0,
            "withdraw ends own-work deferral"
        );
        assert_eq!(
            log.lines()
                .filter(|line| Event::parse_line(line).is_ok_and(|event| event.action == "cancel"))
                .count(),
            1,
            "reuse the existing terminal ledger action"
        );
        let after = rig.log("one");
        assert_ne!(
            rig.invoke("one", &rig.asker, short, "withdraw", REQ, "again")
                .0,
            0
        );
        assert_eq!(
            rig.log("one"),
            after,
            "closed withdrawal refuses without another record"
        );
    }
}

#[test]
fn honeststatus_withdraw_refuses_wrong_seat_even_when_the_display_name_matches() {
    let rig = RequestRig::new("wx", "ask", false);
    rig.write_log(
        &rig.log("one")
            .replace("\"actor\":\"lead\"", "\"actor\":\"helper\""),
    );
    let before = rig.log("one");
    let out = rig.invoke("one", &rig.target, false, "withdraw", REQ, "not mine");
    assert_ne!(out.0, 0, "same display is not asker authority: {out:?}");
    assert_eq!(rig.log("one"), before);
}

#[test]
fn honeststatus_new_helpers_refuse_blank_unknown_or_unidentified_without_mutation() {
    for (tag, helper, actor) in [("wv", "withdraw", false), ("av", "accept", true)] {
        let rig = RequestRig::new(tag, "ask", false);
        let pane = if actor { &rig.target } else { &rig.asker };
        for (id, reason) in [
            (REQ, " \t\n"),
            ("ae-20261020T080000Z-bbbbbbbb", "unknown request"),
        ] {
            let before = rig.log("one");
            let out = rig.invoke("one", pane, false, helper, id, reason);
            assert_eq!(
                out.0,
                if reason.trim().is_empty() { 2 } else { 1 },
                "{helper} rejects invalid input: {out:?}"
            );
            assert_eq!(rig.log("one"), before);
        }
        rig.stamp(pane, "", "");
        let before = rig.log("one");
        assert_ne!(
            rig.invoke("one", pane, true, helper, REQ, "no identity").0,
            0
        );
        assert_eq!(rig.log("one"), before);
    }
}

#[test]
fn honeststatus_accept_keeps_reply_owed_but_removes_unanswered_without_expiry() {
    for (tag, kind, short) in [("aa", "ask", false), ("ar", "review", true)] {
        let rig = RequestRig::new(tag, kind, false);
        let out = rig.invoke("one", &rig.target, short, "accept", REQ, "work in progress");
        assert_eq!(out.0, 0, "target accepts {kind}: {out:?}");
        let read = SessionRead::open(&rig.dir("one")).expect("accepted journal read");
        assert_eq!(read.pending.len(), 1, "accepted work is still owed a reply");
        assert!(read.unanswered(now(), LIMIT).is_empty());
        assert!(
            read.unanswered(Timestamp::from_epoch(NOW + 604_800), LIMIT)
                .is_empty(),
            "acceptance lasts until a terminal event, not a clock guess"
        );
        let ledger = status(&rig.log("one"));
        assert_ne!(ledger, ae::requests::Status::Replied);
        assert_ne!(ledger, ae::requests::Status::Cancelled);
        assert_ne!(ledger, ae::requests::Status::Retired);
        let work = ae::session::Outstanding::read_in(
            &read.events,
            SessionKey::new("one", UUID),
            &["lead".to_owned()],
        );
        assert_eq!(
            work.of(ae::session::Seat::new("one", "main", "lead"))
                .requests,
            1,
            "accepted work remains outstanding"
        );
        let table = rig.run("one", &rig.asker, false, "requests", &["all"]);
        assert_eq!(table.0, 0);
        assert!(
            table.1.contains(REQ) && table.1.contains("accepted"),
            "in-progress status stays visible: {table:?}"
        );
        let before = rig.log("one");
        let duplicate = rig.invoke("one", &rig.target, short, "accept", REQ, "still on it");
        assert_eq!(duplicate.0, 0, "re-accept is idempotent: {duplicate:?}");
        assert_eq!(
            rig.log("one"),
            before,
            "a duplicate cannot mutate the accepted episode"
        );
    }
}

#[test]
fn honeststatus_accept_refuses_asker_or_wrong_target_with_same_display_name() {
    let rig = RequestRig::new("ax", "ask", false);
    let before = rig.log("one");
    assert_ne!(
        rig.invoke("one", &rig.asker, false, "accept", REQ, "not the target")
            .0,
        0
    );
    assert_eq!(rig.log("one"), before);
    rig.write_log(
        &rig.log("one")
            .replace("\"target\":\"helper\"", "\"target\":\"lead\""),
    );
    let before = rig.log("one");
    assert_ne!(
        rig.invoke(
            "one",
            &rig.asker,
            true,
            "accept",
            REQ,
            "same target display"
        )
        .0,
        0
    );
    assert_eq!(rig.log("one"), before);
}

#[test]
fn honeststatus_new_helpers_refuse_a_reused_session_incarnation() {
    for (tag, helper, target) in [("wi", "withdraw", false), ("ai", "accept", true)] {
        let rig = RequestRig::new(tag, "ask", false);
        let path = rig.dir("one").join("meta");
        let meta = std::fs::read_to_string(&path).expect("fixture meta");
        std::fs::write(path, meta.replace(UUID, OTHER_UUID)).expect("synthetic reused session");
        rig.tmux(&["set-option", "-t", "one", "@ae_session_uuid", OTHER_UUID]);
        let before = rig.log("one");
        let pane = if target { &rig.target } else { &rig.asker };
        let out = rig.invoke("one", pane, false, helper, REQ, "old request is not mine");
        assert_ne!(out.0, 0, "{helper} cannot act on old incarnation: {out:?}");
        assert_eq!(rig.log("one"), before);
    }
}

#[test]
fn honeststatus_accept_and_withdraw_keep_cross_session_ledgers_in_agreement() {
    let rig = RequestRig::new("cross", "ask", true);
    let accepted = rig.invoke("two", &rig.target, true, "accept", REQ, "on it remotely");
    assert_eq!(
        accepted.0, 0,
        "cross-session target acceptance: {accepted:?}"
    );
    for session in ["one", "two"] {
        let read = SessionRead::open(&rig.dir(session)).expect("cross participant journal");
        assert_eq!(read.pending.len(), 1, "{session}: accepted is open");
        assert!(
            read.unanswered(now(), LIMIT).is_empty(),
            "{session}: acceptance suppresses unanswered"
        );
    }
    let closed = rig.invoke(
        "one",
        &rig.asker,
        false,
        "withdraw",
        REQ,
        "resolved independently",
    );
    assert_eq!(closed.0, 0, "cross-session asker withdrawal: {closed:?}");
    for session in ["one", "two"] {
        assert_eq!(
            status(&rig.log(session)),
            ae::requests::Status::Cancelled,
            "{session}"
        );
        assert!(
            SessionRead::open(&rig.dir(session))
                .expect("closed participant")
                .pending
                .is_empty()
        );
    }
    let before = [rig.log("one"), rig.log("two")];
    assert_ne!(
        rig.invoke("two", &rig.target, false, "accept", REQ, "too late")
            .0,
        0
    );
    assert_eq!(
        [rig.log("one"), rig.log("two")],
        before,
        "accept cannot reopen a withdrawn request"
    );
}

fn acceptance(slot: &str, uuid: &str, id: &str) -> Event {
    event(
        NOW - 30,
        "helper",
        "accept",
        "lead",
        id,
        &format!(
            ",\"actor_slot\":\"{slot}\",\"actor_session\":\"one\",\"actor_session_id\":\"{uuid}\",\"target_slot\":\"main\",\"target_session\":\"one\",\"target_session_id\":\"{UUID}\""
        ),
    )
}

#[test]
fn honeststatus_acceptance_uses_target_key_and_append_order_not_display_or_timestamp() {
    for accepted in [
        acceptance("main", UUID, REQ),
        acceptance("spawned.0", OTHER_UUID, REQ),
        acceptance("spawned.0", UUID, "ae-20261020T080000Z-bbbbbbbb"),
    ] {
        let read = read(vec![ask("lead", LIMIT + 1), accepted]);
        assert_eq!(read.pending.len(), 1);
        assert_eq!(
            read.unanswered(now(), LIMIT).len(),
            1,
            "foreign acceptance cannot hide the request"
        );
    }
    let before_open = read(vec![
        acceptance("spawned.0", UUID, REQ),
        ask("lead", LIMIT + 1),
    ]);
    assert_eq!(
        before_open.unanswered(now(), LIMIT).len(),
        1,
        "a record before the opening accepts nothing"
    );
    let accepted = read(vec![
        ask("lead", LIMIT + 1),
        acceptance("spawned.0", UUID, REQ),
    ]);
    assert_eq!(accepted.pending.len(), 1);
    assert!(
        accepted.unanswered(now(), LIMIT).is_empty(),
        "valid target acceptance removes request debt"
    );
    let reopened = read(vec![
        ask("lead", LIMIT + 1),
        acceptance("spawned.0", UUID, REQ),
        ask("lead", LIMIT + 1),
    ]);
    assert_eq!(
        reopened.unanswered(now(), LIMIT).len(),
        1,
        "newest opening is a new acceptance episode even when its timestamp is identical"
    );
}

#[test]
fn honeststatus_acceptance_is_not_terminal_and_each_existing_closure_still_works() {
    let reply = event(
        NOW - 10,
        "helper",
        "reply",
        "lead",
        REQ,
        &format!(
            ",\"actor_slot\":\"spawned.0\",\"actor_session\":\"one\",\"actor_session_id\":\"{UUID}\",\"target_slot\":\"main\",\"target_session\":\"one\",\"target_session_id\":\"{UUID}\""
        ),
    );
    let cancel = event(
        NOW - 10,
        "lead",
        "cancel",
        "helper",
        REQ,
        &format!(
            ",\"actor_slot\":\"main\",\"actor_session\":\"one\",\"actor_session_id\":\"{UUID}\",\"target_slot\":\"spawned.0\",\"target_session\":\"one\",\"target_session_id\":\"{UUID}\""
        ),
    );
    let retire = event(
        NOW - 10,
        "lead",
        "retire",
        "helper",
        "",
        ",\"target_slot\":\"spawned.0\"",
    );
    for terminal in [reply, cancel, retire] {
        let read = read(vec![
            ask("lead", LIMIT + 1),
            acceptance("spawned.0", UUID, REQ),
            terminal,
        ]);
        assert!(
            read.pending.is_empty(),
            "acceptance must not interfere with reply/withdraw/retire"
        );
        assert!(read.unanswered(now(), LIMIT).is_empty());
    }
}

#[test]
fn honeststatus_mixed_stale_and_blocked_seats_keep_the_human_mark() {
    let mut entry = entry(Some(Reason::Stale));
    let mut blocked = agent(Some(Reason::Blocked));
    blocked.name = "helper".to_owned();
    blocked.reference = "helper".to_owned();
    entry.agents.push(blocked);
    assert!(
        entry.needs_attention(),
        "stale severity cannot hide a human blocker"
    );
    let world = World::new(now(), vec![entry]);
    let fleet = fleet::rows(
        &world,
        &seats(),
        &BTreeMap::new(),
        &FleetOrder::EMPTY,
        &BTreeMap::new(),
        Some("one"),
        now(),
    );
    assert_eq!(
        fleet.rows[0].mark,
        Mark::NeedsYou,
        "session mark aggregates marks, not severity ranks"
    );
}

fn painted(
    entry: &SessionEntry,
    lane: &ae::console::lane::Lane,
    icons: bool,
) -> ratatui_core::buffer::Buffer {
    use ae::app::draw::{Composer, Screen, draw};
    use ae::app::model::Model;
    use ae::theme::Look;
    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::Rect;
    let world = World::new(now(), vec![entry.clone()]);
    let facts = seats();
    let fleet = fleet::rows(
        &world,
        &facts,
        &BTreeMap::new(),
        &FleetOrder::EMPTY,
        &BTreeMap::new(),
        Some("one"),
        now(),
    );
    let model = Model::new(&fleet);
    let overview = overview::of(entry, None, None, now());
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(entry),
        pair: &["lead".to_owned()],
        agents: facts.get("one"),
        lane,
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
        zone: Some("+0000"),
        now: now(),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
    draw(&screen, &mut buf);
    buf
}

fn buffer_text(buf: &ratatui_core::buffer::Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn honeststatus_app_draws_a_quiet_unanswered_count_in_both_icon_modes() {
    let entry = ae::session::entry_from(
        &snapshot(read(vec![ask("console:local", LIMIT + 1)])),
        "one",
        &runtime(),
        now(),
        LIMIT,
    );
    for icons in [true, false] {
        let buf = painted(&entry, &ae::console::lane::Lane::default(), icons);
        let text = buffer_text(&buf);
        let tally = if icons { "↩1" } else { "r1" };
        assert!(text.contains(tally), "quiet sidebar count: {text}");
        assert!(
            text.contains("unanswered"),
            "selected overview names quiet debt: {text}"
        );
        assert!(
            !text.contains("1 needs you"),
            "request debt is not a human claim: {text}"
        );
    }
}

#[test]
fn honeststatus_partial_journal_does_not_claim_an_exact_zero_unanswered_count() {
    let mut snapshot = snapshot(read(vec![ask("lead", LIMIT + 1)]));
    snapshot.events = None;
    let entry = ae::session::entry_from(&snapshot, "one", &runtime(), now(), LIMIT);
    assert_eq!(
        entry.to_json().get("unanswered"),
        None,
        "unreadable is not exact zero"
    );
}

#[test]
fn honeststatus_cross_session_mirror_failure_is_explicit_and_retry_repairs_original_record() {
    for (tag, helper, caller, peer) in [
        ("repairw", "withdraw", "one", "two"),
        ("repaira", "accept", "two", "one"),
    ] {
        let rig = RequestRig::new(tag, "ask", true);
        let pane = if helper == "withdraw" {
            &rig.asker
        } else {
            &rig.target
        };
        let peer_lock = rig.dir(peer).join("events.jsonl.lock");
        std::fs::create_dir(&peer_lock).expect("owned refusal fixture");
        let out = rig.invoke(caller, pane, false, helper, REQ, "original reason");
        assert_eq!(out.0, 1, "partial mirror reports failure: {out:?}");
        assert!(
            out.2.contains(caller) && out.2.contains(peer) && out.2.contains("recorded"),
            "names progress: {out:?}"
        );
        let action = if helper == "withdraw" {
            "cancel"
        } else {
            "accept"
        };
        let caller_log = rig.log(caller);
        let original = caller_log
            .lines()
            .find(|line| Event::parse_line(line).is_ok_and(|event| event.action == action))
            .expect("caller append landed")
            .to_owned();
        assert!(
            !rig.log(peer)
                .lines()
                .any(|line| Event::parse_line(line).is_ok_and(|event| event.action == action))
        );
        std::fs::remove_dir(peer_lock).expect("restore peer writer");
        let repaired = rig.invoke(caller, pane, true, helper, REQ, "different retry reason");
        assert_eq!(repaired.0, 0, "retry repairs peer: {repaired:?}");
        assert_eq!(
            rig.log(caller),
            caller_log,
            "repair cannot append locally twice"
        );
        assert!(
            rig.log(peer).lines().any(|line| line == original),
            "mirror must preserve original bytes"
        );
        for session in [caller, peer] {
            let log = rig.log(session);
            assert_eq!(
                log.lines()
                    .filter(
                        |line| Event::parse_line(line).is_ok_and(|event| event.action == action)
                    )
                    .count(),
                1
            );
            let read = SessionRead::open(&rig.dir(session)).expect("repaired participant");
            assert!(read.unanswered(now(), LIMIT).is_empty());
            assert_eq!(read.pending.len(), usize::from(helper == "accept"));
        }
        let before = [rig.log(caller), rig.log(peer)];
        let duplicate = rig.invoke(caller, pane, false, helper, REQ, "already converged");
        assert_eq!(duplicate.0, i32::from(helper == "withdraw"));
        assert_eq!([rig.log(caller), rig.log(peer)], before);
    }
}

#[test]
fn honeststatus_caller_append_failure_changes_neither_cross_session_ledger() {
    for (tag, helper, caller) in [("failw", "withdraw", "one"), ("faila", "accept", "two")] {
        let rig = RequestRig::new(tag, "ask", true);
        let pane = if helper == "withdraw" {
            &rig.asker
        } else {
            &rig.target
        };
        let lock = rig.dir(caller).join("events.jsonl.lock");
        std::fs::create_dir(lock).expect("owned caller refusal fixture");
        let before = [rig.log("one"), rig.log("two")];
        let out = rig.invoke(caller, pane, false, helper, REQ, "cannot append");
        assert_eq!(out.0, 1, "caller failure is named: {out:?}");
        assert!(
            out.2.contains("not recorded"),
            "must not claim success: {out:?}"
        );
        assert_eq!([rig.log("one"), rig.log("two")], before);
    }
}

fn console_open() -> Event {
    let mut ask = event(
        NOW - 120,
        "console:local",
        "ask",
        "lead",
        REQ,
        &format!(
            ",\"target_slot\":\"main\",\"target_session\":\"one\",\"target_session_id\":\"{UUID}\",\"target_server\":\"/synthetic/socket\",\"target_pane\":\"%1\",\"target_session_uuid\":\"{UUID}\""
        ),
    );
    ask.summary = Some("human work order".to_owned());
    ask
}

fn console_record(action: &str, note: &str, slot: &str) -> Event {
    let mut record = event(
        NOW - 30,
        "lead",
        action,
        "console:local",
        REQ,
        &format!(
            ",\"actor_slot\":\"{slot}\",\"actor_session\":\"one\",\"actor_session_id\":\"{UUID}\",\"caller_server\":\"/synthetic/socket\",\"caller_pane\":\"%1\",\"caller_session_uuid\":\"{UUID}\""
        ),
    );
    record.summary = Some(note.to_owned());
    record
}

fn console_lane(events: &[Event]) -> ae::console::lane::Lane {
    ae::console::lane::fold(
        "one",
        &[ae::console::lane::Seat {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: None,
        }],
        events,
        &ae::board::Observation::default(),
        0,
        &|_| ae::console::lane::Body::OldCore,
    )
}

#[test]
fn honeststatus_human_work_order_acceptance_renders_once_dim_with_who_note_and_clock() {
    let accepted = console_record("accept", "work in progress", "main");
    let events = vec![console_open(), accepted.clone(), accepted];
    let lane = console_lane(&events);
    assert_eq!(lane.items.len(), 2, "asked turn plus one accepted row");
    assert_eq!(lane.items[1].body, "work in progress");
    assert_eq!(ae::console::lane::open_asks(&events, "one"), [REQ]);
    assert!(
        ae::console::lane::may_close(&events, REQ).is_ok(),
        "accepted is still closeable"
    );
    let text = ae::console::view::Printed::default().step(&lane, 0, true);
    assert_eq!(text.matches("accepted").count(), 1, "{text}");
    assert!(
        text.contains("lead") && text.contains("work in progress"),
        "{text}"
    );
    let stamp = Timestamp::from_epoch(NOW - 30).to_string();
    let clock = &stamp[11..16];
    let accepted_line = text
        .lines()
        .find(|line| line.contains("accepted"))
        .expect("chat acceptance row");
    assert!(
        accepted_line.contains(clock)
            && accepted_line.contains("lead")
            && accepted_line.contains("work in progress"),
        "one chat row holds who, note and the existing clock cell: {text}"
    );
    let buf = painted(&entry(None), &lane, true);
    let text = buffer_text(&buf);
    let accepted_rows: Vec<_> = (0..buf.area.height)
        .filter(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, *y)].symbol())
                .collect::<String>()
                .contains("accepted")
        })
        .collect();
    assert_eq!(accepted_rows.len(), 1, "one app acceptance row: {text}");
    let y = accepted_rows[0];
    let words: String = (47..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
    assert!(
        words.contains("lead") && words.contains("work in progress") && words.contains(clock),
        "{words}"
    );
    let hex = ae::theme::Palette::DARCULA
        .dim
        .strip_prefix('#')
        .expect("palette hex");
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).expect("RGB channel");
    let dim = ratatui_core::style::Color::Rgb(channel(0), channel(2), channel(4));
    for x in 47..buf.area.width {
        if buf[(x, y)].symbol().trim().is_empty() {
            continue;
        }
        assert_eq!(buf[(x, y)].fg, dim, "entire acceptance row is dim: {text}");
    }
}

#[test]
fn honeststatus_chat_ignores_foreign_or_post_terminal_acceptance_and_keeps_real_history() {
    let accepted = console_record("accept", "work in progress", "main");
    let foreign = console_record("accept", "wrong target", "worker.0");
    let lane = console_lane(&[console_open(), foreign]);
    assert_eq!(
        lane.items.len(),
        1,
        "foreign acceptance does not paint progress"
    );
    let reply = console_record("reply", "finished work", "main");
    let cancel = event(NOW - 10, "console:local", "cancel", "lead", REQ, "");
    let retire = event(
        NOW - 10,
        "ae:retire",
        "retire",
        "lead",
        "",
        ",\"target_slot\":\"main\"",
    );
    for terminal in [reply, cancel, retire] {
        let mut after = accepted.clone();
        after.summary = Some("invalid late acceptance".to_owned());
        let lane = console_lane(&[console_open(), terminal.clone(), after]);
        assert!(
            !lane
                .items
                .iter()
                .any(|item| item.body == "invalid late acceptance"),
            "terminal cannot be reopened: {lane:?}"
        );
        let lane = console_lane(&[console_open(), accepted.clone(), terminal]);
        assert_eq!(
            lane.items
                .iter()
                .filter(|item| item.body == "work in progress")
                .count(),
            1,
            "accepted history survives closure"
        );
    }
}

#[test]
fn honeststatus_target_helper_accepts_a_human_work_order_without_closing_it() {
    let rig = RequestRig::new("humanaccept", "ask", false);
    let log = rig
        .log("one")
        .replace("\"actor\":\"lead\"", "\"actor\":\"console:local\"")
        .replace(
            &format!(
                ",\"actor_slot\":\"main\",\"actor_session\":\"one\",\"actor_session_id\":\"{UUID}\""
            ),
            "",
        );
    rig.write_log(&log);
    let out = rig.invoke(
        "one",
        &rig.target,
        false,
        "accept",
        REQ,
        "work in progress for you",
    );
    assert_eq!(out.0, 0, "target accepts external human asker: {out:?}");
    let read = SessionRead::open(&rig.dir("one")).expect("human work order journal");
    assert_eq!(read.pending.len(), 1);
    assert!(read.unanswered(now(), LIMIT).is_empty());
    let card_entry = ae::session::entry_from(&snapshot(read), "one", &runtime(), now(), LIMIT);
    let card = ae::brief::card_for(&card_entry, &rig.dir("one"), None, false, now(), None);
    assert_eq!(
        card.open_ask_count(),
        1,
        "accepted work remains open in brief"
    );
    assert_eq!(card.human_needs().count(), 0);
    assert!(!ae::brief::render(&[card]).contains("unanswered:1"));
}

#[test]
fn honeststatus_acceptance_with_an_older_stamp_still_follows_append_order() {
    let mut accepted = acceptance("spawned.0", UUID, REQ);
    accepted.ts = Timestamp::from_epoch(NOW - 20_000);
    let read = read(vec![ask("lead", LIMIT + 1), accepted]);
    assert_eq!(read.pending.len(), 1);
    assert!(
        read.unanswered(now(), LIMIT).is_empty(),
        "append order is the clock for request state"
    );
}

// Requirement: a mirror reports only the state change its peer actually admits;
// a newer opening belongs to its own routing identity, even when a ref is reused.
#[test]
fn honeststatus_mirror_refuses_a_newer_foreign_opening_without_writing_an_ignored_record() {
    for (tag, helper, caller, peer, party) in [
        ("divergew", "withdraw", "one", "two", "actor_slot"),
        ("divergea", "accept", "two", "one", "target_slot"),
    ] {
        let rig = RequestRig::new(tag, "ask", true);
        let pane = if helper == "withdraw" {
            &rig.asker
        } else {
            &rig.target
        };
        let opening = rig.log(peer);
        let original = if party == "actor_slot" {
            "main"
        } else {
            "spawned.0"
        };
        let newer = opening.replace(
            &format!("\"{party}\":\"{original}\""),
            &format!("\"{party}\":\"spawned.9\""),
        );
        assert_ne!(
            newer, opening,
            "the later opening has another routing party"
        );
        let peer_before = format!("{opening}{newer}");
        std::fs::write(rig.dir(peer).join("events.jsonl"), &peer_before)
            .expect("owned divergent peer journal");
        let out = rig.invoke(caller, pane, false, helper, REQ, "original intent");
        assert_eq!(
            out.0, 1,
            "an ineffective mirror cannot claim success: {out:?}"
        );
        assert!(
            out.2.contains(caller)
                && out.2.contains(peer)
                && out.2.contains("recorded")
                && out.2.contains("failed"),
            "name the caller append and the peer failure: {out:?}"
        );
        assert_eq!(rig.log(peer), peer_before, "never append an ignored mirror");
        let caller_after = rig.log(caller);
        let action = if helper == "withdraw" {
            "cancel"
        } else {
            "accept"
        };
        assert_eq!(
            caller_after
                .lines()
                .filter(|line| {
                    Event::parse_line(line).is_ok_and(|event| event.action == action)
                })
                .count(),
            1,
            "the caller's original record landed once"
        );
        let retry = rig.invoke(caller, pane, true, helper, REQ, "retry intent");
        assert_eq!(
            retry.0, 1,
            "incompatible peer still needs repair: {retry:?}"
        );
        assert_eq!(
            rig.log(peer),
            peer_before,
            "retry cannot append ignored records"
        );
        assert_eq!(
            rig.log(caller),
            caller_after,
            "retry keeps the original record"
        );
    }
}
