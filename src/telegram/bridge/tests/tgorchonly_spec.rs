//! Frozen acceptance specification for slice `tgorchonly` against base `c988d8f01`.
//!
//! Install byte-identically at `src/telegram/bridge/tests/tgorchonly_spec.rs` and wire into
//! `src/telegram/bridge.rs` inside `mod tests` via:
//! `#[path = "tgorchonly_spec.rs"] mod tgorchonly_spec;`
//!
//! Requires two test-only adapters in `bridge::tests`:
//! - `fn relay_from_config(config: &std::path::Path, home: &std::path::Path) -> super::Relay`
//!   built through the same `load_settings()` path as `run()`.
//! - `fn carry_for(relay: &super::Relay, session_id: &str, id: &str)`
//!   populating the same process-local carried-ask set as `SharedChat`.

use super::super::{BACKLOG_FILE, Relay};
use super::{Knobs, Paths, RunningSession, Seat};
use crate::telegram::tests::{Fake, Reply};
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

struct SpecWorld {
    sessions: Vec<RunningSession>,
    seat: Seat,
}

impl crate::telegram::routing::World for SpecWorld {
    fn running(&self) -> Vec<RunningSession> {
        self.sessions.clone()
    }

    fn seat(&self) -> Seat {
        self.seat.clone()
    }
}

struct SpecRoot(PathBuf);

impl SpecRoot {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ae-tgorchonly-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl std::ops::Deref for SpecRoot {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for SpecRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn make_session(name: &str, dir: PathBuf, session_id: &str, meta_agent: bool) -> RunningSession {
    std::fs::create_dir_all(&dir).unwrap();
    RunningSession {
        name: name.to_owned(),
        dir,
        session_id: session_id.to_owned(),
        meta_agent,
        main: Some("lead".to_owned()),
        agents: vec!["lead".to_owned()],
        pair: vec!["lead".to_owned()],
        last_active: None,
    }
}

fn append_event(dir: &Path, line: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(crate::store::EVENTS))
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

fn chat_row(ts: &str, actor: &str, summary: &str) -> String {
    format!(
        r#"{{"ts":"{ts}","actor":"{actor}","action":"chat","ref":"ae-20261119T180000Z-0000abcd","summary":"{summary}"}}"#
    )
}

fn ask_row(ts: &str, reference: &str, summary: &str) -> String {
    let actor = crate::tracked::CONSOLE_SINK;
    format!(
        r#"{{"ts":"{ts}","actor":"{actor}","action":"ask","target":"lead","ref":"{reference}","summary":"{summary}"}}"#
    )
}

fn reply_row(ts: &str, target: &str, reference: &str, summary: &str) -> String {
    format!(
        r#"{{"ts":"{ts}","actor":"lead","action":"reply","target":"{target}","ref":"{reference}","summary":"{summary}"}}"#
    )
}

fn sent_texts(fake: &Fake) -> Vec<String> {
    fake.requests()
        .iter()
        .map(|seen| crate::json::parse(&seen.body).unwrap())
        .map(|value| value.get_str("text").unwrap_or_default().to_owned())
        .collect()
}

fn write_token_and_config(root: &Path, extra_telegram_lines: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    let home = root.join("home");
    let ae_home = home.join(".ae");
    std::fs::create_dir_all(&ae_home).unwrap();
    let token = root.join("bot.token");
    std::fs::write(&token, "123456:TESTTOKEN\n").unwrap();
    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
    let config = ae_home.join("config");
    let content = format!(
        "[telegram]\nenabled = true\ntoken_file = \"{}\"\nchat_id = 4242\n{extra_telegram_lines}",
        token.display()
    );
    std::fs::write(&config, content).unwrap();
    (home, config)
}

fn seed_orch_and_worker_events(orch_dir: &Path, worker_dir: &Path) {
    for line in [
        chat_row("2026-11-19T18:00:00Z", "lead", "ORCH-SAY"),
        chat_row(
            "2026-11-19T18:00:01Z",
            "ae:relay",
            "routed to worker: ask (sent), request r-1",
        ),
        chat_row(
            "2026-11-19T18:00:02Z",
            "ae:relay",
            "reply fd5bf0 from lead: ORCH-RAW-ECHO",
        ),
        ask_row(
            "2026-11-19T18:00:03Z",
            "ae-20261119T180003Z-00000001",
            "ORCH-ASK",
        ),
        reply_row(
            "2026-11-19T18:00:04Z",
            crate::tracked::CONSOLE_SINK,
            "ae-20261119T180003Z-00000001",
            "ORCH-REPLY",
        ),
    ] {
        append_event(orch_dir, &line);
    }
    for line in [
        chat_row("2026-11-19T18:00:00Z", "lead", "WORKER-SAY"),
        chat_row("2026-11-19T18:00:01Z", "watchdog", "WORKER-WD"),
        chat_row("2026-11-19T18:00:02Z", "ae:notice", "WORKER-AE"),
        ask_row(
            "2026-11-19T18:00:03Z",
            "ae-20261119T180003Z-00000002",
            "WORKER-ASK",
        ),
        reply_row(
            "2026-11-19T18:00:04Z",
            crate::tracked::CONSOLE_SINK,
            "ae-20261119T180003Z-00000002",
            "WORKER-REPLY",
        ),
    ] {
        append_event(worker_dir, &line);
    }
}

#[test]
fn orchestrator_only_drops_other_sessions_and_keeps_orchestrator_preserving_orchquiet() {
    let root = SpecRoot::new("orch-only");
    let orch_dir = root.join("orch");
    let worker_dir = root.join("worker");
    let orch = make_session(
        "orch",
        orch_dir.clone(),
        "01950000-0000-7000-8000-000000000001",
        true,
    );
    let worker = make_session(
        "worker",
        worker_dir.clone(),
        "01950000-0000-7000-8000-000000000002",
        false,
    );
    seed_orch_and_worker_events(&orch_dir, &worker_dir);

    let world = SpecWorld {
        sessions: vec![orch, worker],
        seat: Seat::Held {
            name: "orch".to_owned(),
            dir: orch_dir,
        },
    };
    let fake = Fake::start(vec![Reply::ok()]);
    let mut bridges = BTreeMap::new();
    let mut err = Vec::new();
    super::super::forward(
        &fake.api(),
        &world,
        &super::relay(),
        &root,
        &mut bridges,
        &mut err,
    )
    .unwrap();

    let texts = sent_texts(&fake);
    for expected in ["ORCH-SAY", "routed to worker", "ORCH-ASK", "ORCH-REPLY"] {
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with("[orch] ") && t.contains(expected)),
            "{texts:?}"
        );
    }
    assert!(
        !texts.iter().any(|t| t.contains("ORCH-RAW-ECHO")),
        "orchquiet must still hide raw relay answer echoes: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.starts_with("[worker] ")),
        "other-session rows must be dropped while an orchestrator seat is proven: {texts:?}"
    );
}

#[test]
fn canonical_dir_match_rejects_same_name_worktree_and_stopped_seat_still_filters() {
    let root = SpecRoot::new("canon-stopped");
    let canonical_orch = root.join("canonical").join("orch");
    let worktree_orch = root.join("wt").join("orch");
    let other_dir = root.join("other");

    let impostor = make_session(
        "orch",
        worktree_orch.clone(),
        "01950000-0000-7000-8000-000000000010",
        true,
    );
    let other = make_session(
        "other",
        other_dir.clone(),
        "01950000-0000-7000-8000-000000000011",
        false,
    );

    append_event(
        &worktree_orch,
        &chat_row("2026-11-19T18:00:00Z", "lead", "IMPOSTOR-SAY"),
    );
    append_event(
        &other_dir,
        &chat_row("2026-11-19T18:00:00Z", "lead", "OTHER-SAY"),
    );

    // Proven seat is canonical_orch (which is stopped / absent from running).
    // `impostor` shares the name "orch" and even has meta_agent=true, but its dir differs.
    let world = SpecWorld {
        sessions: vec![impostor, other],
        seat: Seat::Held {
            name: "orch".to_owned(),
            dir: canonical_orch,
        },
    };
    let fake = Fake::start(vec![Reply::ok()]);
    let mut bridges = BTreeMap::new();
    let mut err = Vec::new();
    super::super::forward(
        &fake.api(),
        &world,
        &super::relay(),
        &root,
        &mut bridges,
        &mut err,
    )
    .unwrap();

    let texts = sent_texts(&fake);
    assert!(
        texts.is_empty(),
        "same-name worktree and other sessions must be dropped when canonical seat is stopped: {texts:?}"
    );
}

#[test]
fn carried_telegram_ask_answer_is_forwarded_only_in_its_session_while_ask_and_uncarried_stay_dropped()
 {
    let root = SpecRoot::new("carried-reply");
    let orch_dir = root.join("orch");
    let other_dir = root.join("other");
    let third_dir = root.join("third");
    let orch = make_session(
        "orch",
        orch_dir.clone(),
        "01950000-0000-7000-8000-000000000020",
        true,
    );
    let other = make_session(
        "other",
        other_dir.clone(),
        "01950000-0000-7000-8000-000000000021",
        false,
    );
    let third = make_session(
        "third",
        third_dir.clone(),
        "01950000-0000-7000-8000-000000000022",
        false,
    );

    let carried_id = "ae-20261119T180000Z-00000021";
    let uncarried_id = "ae-20261119T180002Z-00000022";

    append_event(
        &other_dir,
        &ask_row("2026-11-19T18:00:00Z", carried_id, "ASK-ECHO"),
    );
    append_event(
        &other_dir,
        &reply_row(
            "2026-11-19T18:00:01Z",
            crate::tracked::CONSOLE_SINK,
            carried_id,
            "CARRIED-ANSWER",
        ),
    );
    append_event(
        &other_dir,
        &ask_row("2026-11-19T18:00:01Z", uncarried_id, "UNCARRIED-ASK"),
    );
    append_event(
        &other_dir,
        &reply_row(
            "2026-11-19T18:00:02Z",
            crate::tracked::CONSOLE_SINK,
            uncarried_id,
            "UNCARRIED-ANSWER",
        ),
    );
    append_event(
        &other_dir,
        &chat_row("2026-11-19T18:00:03Z", "lead", "OTHER-CHAT"),
    );

    // Same ask id in a different session must NOT be admitted by `carry_for(..., other, carried_id)`.
    append_event(
        &third_dir,
        &ask_row("2026-11-19T18:00:03Z", carried_id, "WRONG-SESSION-ASK"),
    );
    append_event(
        &third_dir,
        &reply_row(
            "2026-11-19T18:00:04Z",
            crate::tracked::CONSOLE_SINK,
            carried_id,
            "WRONG-SESSION-ANSWER",
        ),
    );

    let relay = super::relay();
    super::carry_for(&relay, &other.session_id, carried_id);

    let world = SpecWorld {
        sessions: vec![orch, other, third],
        seat: Seat::Held {
            name: "orch".to_owned(),
            dir: orch_dir,
        },
    };
    let fake = Fake::start(vec![Reply::ok()]);
    let mut bridges = BTreeMap::new();
    let mut err = Vec::new();
    super::super::forward(&fake.api(), &world, &relay, &root, &mut bridges, &mut err).unwrap();

    let texts = sent_texts(&fake);
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert!(
        texts[0].starts_with("[other] ") && texts[0].contains("CARRIED-ANSWER"),
        "{texts:?}"
    );
}

#[test]
fn vacant_and_unproven_seat_fall_back_to_all_sessions() {
    for (tag, seat) in [
        ("vacant", Seat::Vacant),
        (
            "unproven",
            Seat::Unproven("two seats claim orchestrator".to_owned()),
        ),
    ] {
        let root = SpecRoot::new(tag);
        let orch_dir = root.join("orch");
        let other_dir = root.join("other");
        let orch = make_session(
            "orch",
            orch_dir.clone(),
            "01950000-0000-7000-8000-000000000030",
            true,
        );
        let other = make_session(
            "other",
            other_dir.clone(),
            "01950000-0000-7000-8000-000000000031",
            false,
        );
        append_event(
            &orch_dir,
            &chat_row("2026-11-19T18:00:00Z", "lead", "ORCH-LINE"),
        );
        append_event(
            &other_dir,
            &chat_row("2026-11-19T18:00:00Z", "lead", "OTHER-LINE"),
        );

        let world = SpecWorld {
            sessions: vec![orch, other],
            seat,
        };
        let fake = Fake::start(vec![Reply::ok()]);
        let mut bridges = BTreeMap::new();
        let mut err = Vec::new();
        super::super::forward(
            &fake.api(),
            &world,
            &super::relay(),
            &root,
            &mut bridges,
            &mut err,
        )
        .unwrap();

        let texts = sent_texts(&fake);
        assert!(
            texts.iter().any(|t| t.contains("ORCH-LINE")),
            "{tag}: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("OTHER-LINE")),
            "{tag}: {texts:?}"
        );
    }
}

#[test]
fn census_refreshes_each_pass_and_filtered_rows_checkpoint_without_backlog() {
    let root = SpecRoot::new("checkpoint-backlog");
    let orch_dir = root.join("orch");
    let other_dir = root.join("other");
    let orch = make_session(
        "orch",
        orch_dir.clone(),
        "01950000-0000-7000-8000-000000000040",
        true,
    );
    let other = make_session(
        "other",
        other_dir.clone(),
        "01950000-0000-7000-8000-000000000041",
        false,
    );

    // Cutoff at 2026-11-19T18:00:00Z = 1_795_111_200.
    let mut relay = super::relay();
    relay.since = 1_795_111_200;

    // Stale row + current row in `other` while orchestrator seat is held.
    append_event(
        &other_dir,
        &chat_row("2026-11-19T17:00:00Z", "lead", "STALE-FILTERED"),
    );
    append_event(
        &other_dir,
        &chat_row("2026-11-19T18:05:00Z", "lead", "DROPPED-PASS1"),
    );

    let mut world = SpecWorld {
        sessions: vec![orch, other],
        seat: Seat::Held {
            name: "orch".to_owned(),
            dir: orch_dir,
        },
    };
    let fake = Fake::start(vec![Reply::ok()]);
    let mut bridges = BTreeMap::new();
    let mut err = Vec::new();
    let stale_pass1 =
        super::super::forward(&fake.api(), &world, &relay, &root, &mut bridges, &mut err).unwrap();
    assert_eq!(
        stale_pass1, 0,
        "scoped-out stale rows must not trigger backlog count"
    );
    assert!(
        !root.join(BACKLOG_FILE).exists(),
        "scoped-out stale rows must not write backlog.log"
    );
    assert!(sent_texts(&fake).is_empty());

    // Pass 2: seat becomes Vacant and `other` emits a new row.
    world.seat = Seat::Vacant;
    append_event(
        &other_dir,
        &chat_row("2026-11-19T18:06:00Z", "lead", "SHOWN-PASS2"),
    );
    super::super::forward(&fake.api(), &world, &relay, &root, &mut bridges, &mut err).unwrap();

    let texts = sent_texts(&fake);
    assert!(
        !texts.iter().any(|t| t.contains("DROPPED-PASS1")),
        "checkpointed filtered row must not replay when scope widens: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("SHOWN-PASS2")),
        "new row after census change must forward: {texts:?}"
    );
}

fn forward_with_config(
    root: &Path,
    extra: &str,
    world: &SpecWorld,
    bridges: &mut BTreeMap<PathBuf, crate::telegram::Outbound>,
) -> Vec<String> {
    let (home, config) = write_token_and_config(root, extra);
    let relay: Relay = super::relay_from_config(&config, &home);
    let fake = Fake::start(vec![Reply::ok()]);
    let mut err = Vec::new();
    super::super::forward(&fake.api(), world, &relay, root, bridges, &mut err).unwrap();
    sent_texts(&fake)
}

fn build_config_scope_world(root: &Path) -> (SpecWorld, [PathBuf; 6]) {
    let dirs = [
        root.join("orch"),
        root.join("alpha"),
        root.join("beta_upper"),
        root.join("beta_lower"),
        root.join("dup1"),
        root.join("dup2"),
    ];
    let sessions = vec![
        make_session(
            "orch",
            dirs[0].clone(),
            "01950000-0000-7000-8000-000000000050",
            true,
        ),
        make_session(
            "alpha",
            dirs[1].clone(),
            "01950000-0000-7000-8000-00000000005A",
            false,
        ),
        make_session(
            "Beta",
            dirs[2].clone(),
            "01950000-0000-7000-8000-000000000052",
            false,
        ),
        make_session(
            "beta",
            dirs[3].clone(),
            "01950000-0000-7000-8000-000000000053",
            false,
        ),
        make_session(
            "dup",
            dirs[4].clone(),
            "01950000-0000-7000-8000-000000000054",
            false,
        ),
        make_session(
            "dup",
            dirs[5].clone(),
            "01950000-0000-7000-8000-000000000055",
            false,
        ),
    ];
    let world = SpecWorld {
        sessions,
        seat: Seat::Held {
            name: "orch".to_owned(),
            dir: dirs[0].clone(),
        },
    };
    (world, dirs)
}

#[test]
fn config_scope_and_also_allow_all_or_unique_selected_sessions() {
    let root = SpecRoot::new("config-scope-also");
    let (
        world,
        [
            orch_dir,
            alpha_dir,
            beta_upper_dir,
            beta_lower_dir,
            dup1_dir,
            dup2_dir,
        ],
    ) = build_config_scope_world(&root);
    let mut bridges = BTreeMap::new();

    // Subcase A: scope = orchestrator + also with uppercase UUID (case-insensitive) + exact name "Beta" + ambiguous "dup".
    for (dir, label) in [
        (&orch_dir, "ORCH-A"),
        (&alpha_dir, "ALPHA-A"),
        (&beta_upper_dir, "BETA-UPPER-A"),
        (&beta_lower_dir, "BETA-LOWER-A"),
        (&dup1_dir, "DUP1-A"),
        (&dup2_dir, "DUP2-A"),
    ] {
        append_event(dir, &chat_row("2026-11-19T18:00:00Z", "lead", label));
    }
    let texts = forward_with_config(
        &root,
        "scope = orchestrator\nalso = 01950000-0000-7000-8000-00000000005A, Beta, dup\n",
        &world,
        &mut bridges,
    );
    for expected in ["ORCH-A", "ALPHA-A", "BETA-UPPER-A"] {
        assert!(texts.iter().any(|t| t.contains(expected)), "{texts:?}");
    }
    for dropped in ["BETA-LOWER-A", "DUP1-A", "DUP2-A"] {
        assert!(!texts.iter().any(|t| t.contains(dropped)), "{texts:?}");
    }

    // Subcase B: scope = all forwards every session.
    append_event(
        &beta_lower_dir,
        &chat_row("2026-11-19T18:01:00Z", "lead", "BETA-LOWER-ALL"),
    );
    append_event(
        &dup1_dir,
        &chat_row("2026-11-19T18:01:00Z", "lead", "DUP1-ALL"),
    );
    let texts_all = forward_with_config(&root, "scope = all\n", &world, &mut bridges);
    assert!(
        texts_all.iter().any(|t| t.contains("BETA-LOWER-ALL")),
        "{texts_all:?}"
    );
    assert!(
        texts_all.iter().any(|t| t.contains("DUP1-ALL")),
        "{texts_all:?}"
    );

    // Subcase C: blank `also =` permits none beyond orchestrator.
    append_event(
        &alpha_dir,
        &chat_row("2026-11-19T18:02:00Z", "lead", "ALPHA-BLANK"),
    );
    append_event(
        &orch_dir,
        &chat_row("2026-11-19T18:02:00Z", "lead", "ORCH-BLANK"),
    );
    let texts_blank = forward_with_config(&root, "also =\n", &world, &mut bridges);
    assert!(
        texts_blank.iter().any(|t| t.contains("ORCH-BLANK")),
        "{texts_blank:?}"
    );
    assert!(
        !texts_blank.iter().any(|t| t.contains("ALPHA-BLANK")),
        "{texts_blank:?}"
    );
}

#[test]
fn config_refuses_malformed_scope_and_also_with_file_line_and_run_fails_before_network() {
    let cases = [
        ("upper-scope", "scope = ORCHESTRATOR\n", 5),
        ("bad-scope", "scope = bogus\n", 5),
        ("empty-middle", "also = alpha,,beta\n", 5),
        ("trailing-comma", "also = alpha,\n", 5),
        ("bad-selector", "also = alpha/beta\n", 5),
        ("repeat-scope", "scope = all\nscope = orchestrator\n", 6),
        ("repeat-also", "also = alpha\nalso = beta\n", 6),
    ];
    for (tag, extra, expected_line) in cases {
        let root = SpecRoot::new(tag);
        let (home, config) = write_token_and_config(&root, extra);
        let Err(err) = crate::telegram::load_settings(&config, &home) else {
            panic!("{tag} must be refused");
        };
        let msg = err.to_string();
        assert_eq!(
            msg.lines().count(),
            1,
            "{tag}: expected single diagnostic line, got {msg:?}"
        );
        let needle = format!("{}:{expected_line}", config.display());
        assert!(msg.contains(&needle), "{tag}: expected {needle} in {msg:?}");

        let paths = Paths::under(home.join(".ae"));
        let mut run_err = Vec::new();
        let rc = super::super::run(
            &paths,
            Knobs::default(),
            crate::settings_menu::role_holder,
            &mut run_err,
        )
        .unwrap();
        assert_eq!(rc, 1, "{tag}");
        let stderr = String::from_utf8_lossy(&run_err);
        assert_eq!(
            stderr.lines().count(),
            1,
            "{tag}: expected single stderr line, got {stderr:?}"
        );
        assert!(
            stderr.contains(&needle),
            "{tag}: expected {needle} in {stderr:?}"
        );
    }
}
