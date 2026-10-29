//! #242 slice 1 — live seat facts in `ae list --json`: `client`, `account`,
//! `runtime` per agent and `coverage` per session.
//!
//! ORACLE: issue #242 and lead rulings R1-R6 as amended (2026-10-23): the
//! existing `observed` string IS the activity axis and is not duplicated;
//! `client` and `runtime` come ONLY from a fresh, strictly parsed `@ae_agents`
//! fact whose entry is proven to be this seat (name, recorded profile, the one
//! listed pane of its slot, a non-dead verdict); `account` is the recorded
//! identity join (claude, codex) or `null`; `coverage` counts live seats of a
//! running session. Every unavailable cell reads `"unknown"`, never a profile or
//! meta guess. New keys append after every existing key.
//!
//! Two layers: the agreed seam `ae::session::enrich_list_json`, asserted only
//! through `SessionEntry::to_json`, and the real `ae list --all --json` against
//! a private tmux server whose `@ae_*` facts are set by hand (no watchdog).
use std::fs;
use std::path::{Path, PathBuf};

use ae::digest::{AgentEntry, SessionEntry, Status};
use ae::json::Value;
use ae::meta::Meta;
use ae::time::Timestamp;
use ae::tmux::WatchPane;

use super::parity::Invocation;
use super::parity::capture::raw;
use super::phase2::{run_tmux, tmux_present};

const NOW: i64 = 2_000_000;

/// The existing agent keys, in their published order (docs/reference/commands.md).
const EXISTING_AGENT_KEYS: [&str; 8] = [
    "ref",
    "alias",
    "name",
    "session_id",
    "alive",
    "observed",
    "state",
    "reason",
];

const NEW_AGENT_KEYS: [&str; 4] = ["client", "account", "runtime", "conversation"];

// ---------------------------------------------------------------------------
// seam fixtures
// ---------------------------------------------------------------------------

fn roster(meta: &str) -> Vec<ae::meta::RosterEntry> {
    Meta::parse(meta).roster().to_vec()
}

/// One seat `lead` on profile `cl`, a claude binary and no recorded home.
const LEAD: &str = "seat.main=lead\nprofile.main=cl\nagent_bin.main=claude\n";

/// `lead` plus a second seat `builder` on profile `cx` (codex).
const PAIR: &str = "seat.main=lead\nprofile.main=cl\nagent_bin.main=claude\n\
                    seat.worker.0=builder\nprofile.worker.0=cx\nagent_bin.worker.0=codex\n";

fn pane(id: &str, slot: &str, name: &str) -> WatchPane {
    WatchPane {
        pane_id: id.to_owned(),
        slot: Some(slot.to_owned()),
        agent: Some(name.to_owned()),
        current_command: "claude".to_owned(),
        pane_pid: Some(4321),
        observed: "busy".to_owned(),
    }
}

/// A session entry holding one agent entry per roster seat, by name.
fn session(
    status: Status,
    roster: &[ae::meta::RosterEntry],
    alive: &[Option<bool>],
) -> SessionEntry {
    let mut entry = SessionEntry::new("spec", status);
    entry.agents = roster
        .iter()
        .zip(alive.iter().copied().chain(std::iter::repeat(Some(true))))
        .map(|(seat, alive)| AgentEntry {
            reference: seat.name.clone(),
            alias: seat.profile.clone().unwrap_or_default(),
            name: seat.name.clone(),
            alive,
            ..AgentEntry::default()
        })
        .collect();
    entry
}

fn enrich(
    status: Status,
    meta: &str,
    alive: &[Option<bool>],
    fact: Option<&str>,
    panes: Option<&[WatchPane]>,
    now: i64,
) -> Value {
    let roster = roster(meta);
    let mut entry = session(status, &roster, alive);
    ae::session::enrich_list_json(&mut entry, &roster, fact, panes, Timestamp::from_epoch(now));
    entry.to_json()
}

fn agents(value: &Value) -> &[Value] {
    match value.get("agents") {
        Some(Value::Arr(agents)) => agents,
        other => panic!("agents[] missing: {other:?} in {}", value.render()),
    }
}

fn agent<'a>(value: &'a Value, name: &str) -> &'a Value {
    agents(value)
        .iter()
        .find(|agent| agent.get_str("name") == Some(name))
        .unwrap_or_else(|| panic!("no agent {name} in {}", value.render()))
}

fn keys(value: &Value) -> Vec<&str> {
    match value {
        Value::Obj(fields) => fields.iter().map(|(key, _)| key.as_str()).collect(),
        other => panic!("not an object: {}", other.render()),
    }
}

/// `(client, model, effort)` as rendered.
fn live(agent: &Value) -> (String, String, String) {
    let client = agent.get_str("client").map(ToOwned::to_owned);
    let runtime = agent
        .get("runtime")
        .unwrap_or_else(|| panic!("runtime missing: {}", agent.render()));
    let cell = |key: &str| {
        runtime
            .get_str(key)
            .unwrap_or_else(|| panic!("runtime.{key} is not a string: {}", agent.render()))
            .to_owned()
    };
    (
        client.unwrap_or_else(|| panic!("client is not a string: {}", agent.render())),
        cell("model"),
        cell("effort"),
    )
}

fn unknown() -> (String, String, String) {
    ("unknown".into(), "unknown".into(), "unknown".into())
}

fn triple(client: &str, model: &str, effort: &str) -> (String, String, String) {
    (client.into(), model.into(), effort.into())
}

fn fact_at(epoch: i64, interval: u64, entries: &str) -> String {
    format!("v2;{epoch};{interval};{entries}")
}

fn full(entries: &str) -> String {
    fact_at(NOW, 60, entries)
}

fn coverage(value: &Value) -> Option<(i64, i64, i64)> {
    match value.get("coverage") {
        Some(Value::Null) => None,
        Some(object @ Value::Obj(_)) => {
            let n = |key: &str| match object.get(key) {
                Some(Value::Num(n)) => *n,
                other => panic!("coverage.{key}: {other:?} in {}", value.render()),
            };
            assert_eq!(
                keys(object),
                ["joined_live_seats", "live_seats", "unknown_seats"],
                "coverage key order"
            );
            Some((n("joined_live_seats"), n("live_seats"), n("unknown_seats")))
        }
        other => panic!(
            "coverage missing or mistyped: {other:?} in {}",
            value.render()
        ),
    }
}

// ---------------------------------------------------------------------------
// seam: shape
// ---------------------------------------------------------------------------

#[test]
fn new_agent_keys_append_after_every_existing_key_and_coverage_closes_the_session() {
    let panes = [pane("%3", "main", "lead")];
    let value = enrich(
        Status::Running,
        LEAD,
        &[Some(true)],
        Some(&full("lead:cl:working:%3:claude:Spec Model 9:xhigh:")),
        Some(&panes),
        NOW,
    );
    let lead = agent(&value, "lead");
    let mut expected: Vec<&str> = EXISTING_AGENT_KEYS.to_vec();
    expected.extend(NEW_AGENT_KEYS);
    assert_eq!(keys(lead), expected, "{}", value.render());
    assert_eq!(
        keys(lead.get("runtime").expect("runtime")),
        ["model", "effort"]
    );
    assert_eq!(
        keys(&value).last().copied(),
        Some("coverage"),
        "{}",
        value.render()
    );
    // Activity stays the existing string, untouched by the fact's verdict word.
    assert_eq!(
        lead.get_str("observed"),
        Some("unknown"),
        "{}",
        value.render()
    );
}

#[test]
fn new_agent_keys_stay_last_when_state_and_reason_are_absent() {
    // An incomplete event source drops `state`/`reason`; the new keys must not
    // depend on them and still close the object.
    let roster = roster(LEAD);
    let mut entry = SessionEntry::degraded("spec", Status::Running);
    entry.agents = session(Status::Running, &roster, &[Some(true)]).agents;
    ae::session::enrich_list_json(&mut entry, &roster, None, None, Timestamp::from_epoch(NOW));
    let value = entry.to_json();
    let lead = agent(&value, "lead");
    let found = keys(lead);
    assert!(
        !found.contains(&"state"),
        "fixture premise: {}",
        value.render()
    );
    assert_eq!(
        &found[found.len() - 4..],
        NEW_AGENT_KEYS,
        "{}",
        value.render()
    );
    assert_eq!(
        keys(&value).last().copied(),
        Some("coverage"),
        "after degraded too"
    );
}

// ---------------------------------------------------------------------------
// seam: client and runtime from the fresh fact only
// ---------------------------------------------------------------------------

#[test]
fn a_fresh_full_v2_entry_proven_to_be_the_seat_supplies_client_model_and_effort() {
    let panes = [pane("%3", "main", "lead")];
    let value = enrich(
        Status::Running,
        LEAD,
        &[Some(true)],
        Some(&full("lead:cl:working:%3:cc-mic:Spec Model 9:xhigh:!")),
        Some(&panes),
        NOW,
    );
    assert_eq!(
        live(agent(&value, "lead")),
        triple("cc-mic", "Spec Model 9", "xhigh")
    );
}

#[test]
fn the_fact_verdict_word_never_gates_runtime_except_dead() {
    let panes = [pane("%3", "main", "lead")];
    for state in [
        "working",
        "idle",
        "done",
        "waiting-agent",
        "blocked",
        "limit",
        "stale",
    ] {
        let value = enrich(
            Status::Running,
            LEAD,
            &[Some(true)],
            Some(&full(&format!(
                "lead:cl:{state}:%3:claude:Spec Model 9:high:"
            ))),
            Some(&panes),
            NOW,
        );
        assert_eq!(
            live(agent(&value, "lead")),
            triple("claude", "Spec Model 9", "high"),
            "verdict {state}"
        );
    }
    let value = enrich(
        Status::Running,
        LEAD,
        &[Some(true)],
        Some(&full("lead:cl:dead:%3:claude:Spec Model 9:high:")),
        Some(&panes),
        NOW,
    );
    assert_eq!(
        live(agent(&value, "lead")),
        unknown(),
        "a dead fact proves nothing live"
    );
}

#[test]
fn degraded_rungs_leave_exactly_the_cells_they_dropped_unknown() {
    let panes = [pane("%3", "main", "lead")];
    let read = |fact: &str| {
        live(agent(
            &enrich(
                Status::Running,
                LEAD,
                &[Some(true)],
                Some(fact),
                Some(&panes),
                NOW,
            ),
            "lead",
        ))
    };
    // NoEffort rung: a model with no effort cell.
    assert_eq!(
        read(&full("lead:cl:working:%3:claude:Spec Model 9::")),
        triple("claude", "Spec Model 9", "unknown")
    );
    // ClientOnly rung: the client and nothing observed.
    assert_eq!(
        read(&full("lead:cl:working:%3:claude:::")),
        triple("claude", "unknown", "unknown")
    );
    // v1 (Legacy rung): no observed cells and no client.
    assert_eq!(read(&fact_at(NOW, 60, "lead:cl:working:%3")), unknown());
}

#[test]
fn the_writers_no_label_dash_reads_unknown_and_a_legacy_token_is_verbatim() {
    let panes = [pane("%3", "main", "lead")];
    let read = |fact: &str| {
        live(agent(
            &enrich(
                Status::Running,
                LEAD,
                &[Some(true)],
                Some(fact),
                Some(&panes),
                NOW,
            ),
            "lead",
        ))
    };
    assert_eq!(read(&full("lead:cl:working:%3:-:::")), unknown());
    assert_eq!(
        read(&full("lead:cl:working:%3:cx:::")),
        triple("cx", "unknown", "unknown")
    );
}

#[test]
fn freshness_is_two_own_intervals_back_and_one_ahead_of_the_listing_clock() {
    let panes = [pane("%3", "main", "lead")];
    let entries = "lead:cl:working:%3:claude:Spec Model 9:high:";
    let read = |epoch: i64| {
        live(agent(
            &enrich(
                Status::Running,
                LEAD,
                &[Some(true)],
                Some(&fact_at(epoch, 60, entries)),
                Some(&panes),
                NOW,
            ),
            "lead",
        ))
    };
    let seen = triple("claude", "Spec Model 9", "high");
    assert_eq!(read(NOW - 120), seen, "exactly two intervals old is fresh");
    assert_eq!(
        read(NOW - 121),
        unknown(),
        "older than two intervals is stale"
    );
    assert_eq!(read(NOW + 60), seen, "one interval ahead is tolerated");
    assert_eq!(read(NOW + 61), unknown(), "further ahead is untrusted");
}

/// One input pair and why it proves nothing.
type Case<'a> = (Option<&'a str>, Option<&'a [WatchPane]>, &'a str);

#[test]
fn an_absent_or_malformed_fact_or_listing_leaves_client_and_runtime_unknown() {
    let panes = [pane("%3", "main", "lead")];
    let good = full("lead:cl:working:%3:claude:Spec Model 9:high:");
    let cases: [Case<'_>; 5] = [
        (None, Some(&panes), "no fact"),
        (
            Some("v3;2000000;60;lead:cl:working:%3"),
            Some(&panes),
            "unknown version",
        ),
        (
            Some("v2;2000000;60;lead:cl:working:%3:claude:Spec Model 9"),
            Some(&panes),
            "arity",
        ),
        (Some("garbage"), Some(&panes), "garbage"),
        (Some(good.as_str()), None, "no pane listing"),
    ];
    for (fact, listed, why) in cases {
        let value = enrich(Status::Running, LEAD, &[Some(true)], fact, listed, NOW);
        assert_eq!(live(agent(&value, "lead")), unknown(), "{why}");
    }
}

#[test]
fn an_entry_not_proven_to_be_this_seat_is_never_attributed_to_it() {
    let entries = "lead:cl:working:%3:claude:Spec Model 9:high:";
    let read = |fact: &str, listed: &[WatchPane]| {
        live(agent(
            &enrich(
                Status::Running,
                LEAD,
                &[Some(true)],
                Some(fact),
                Some(listed),
                NOW,
            ),
            "lead",
        ))
    };
    let main = [pane("%3", "main", "lead")];
    // The control: proven.
    assert_eq!(
        read(&full(entries), &main),
        triple("claude", "Spec Model 9", "high")
    );
    // A profile the roster no longer records (a reseat inside two intervals).
    assert_eq!(
        read(
            &full("lead:old:working:%3:claude:Spec Model 9:high:"),
            &main
        ),
        unknown(),
        "profile mismatch"
    );
    // The roster seat is absent from the fact.
    assert_eq!(
        read(
            &full("other:cl:working:%3:claude:Spec Model 9:high:"),
            &main
        ),
        unknown(),
        "name absent"
    );
    // The fact names another pane than the one listed for the slot.
    assert_eq!(
        read(&full("lead:cl:working:%9:claude:Spec Model 9:high:"), &main),
        unknown(),
        "pane mismatch"
    );
    // The fact carries no pane.
    assert_eq!(
        read(&full("lead:cl:working::claude:Spec Model 9:high:"), &main),
        unknown(),
        "empty pane"
    );
    // The slot is listed on two panes: no unique pane to prove.
    assert_eq!(
        read(
            &full(entries),
            &[pane("%3", "main", "lead"), pane("%4", "main", "lead")]
        ),
        unknown(),
        "duplicate slot"
    );
    // The slot is not listed at all.
    assert_eq!(
        read(&full(entries), &[pane("%3", "worker.0", "lead")]),
        unknown(),
        "unlisted slot"
    );
}

#[test]
fn each_seat_is_proven_on_its_own() {
    let panes = [
        pane("%3", "main", "lead"),
        pane("%5", "worker.0", "builder"),
    ];
    let value = enrich(
        Status::Running,
        PAIR,
        &[Some(true), Some(true)],
        Some(&full(
            "lead:cl:working:%3:claude:Spec Model 9:high:;builder:old:working:%5:cx:Other Model:low:",
        )),
        Some(&panes),
        NOW,
    );
    assert_eq!(
        live(agent(&value, "lead")),
        triple("claude", "Spec Model 9", "high")
    );
    assert_eq!(
        live(agent(&value, "builder")),
        unknown(),
        "builder's fact names a stale profile"
    );
}

#[test]
fn a_session_that_is_not_running_reports_no_live_cells_even_with_a_fresh_fact() {
    let panes = [pane("%3", "main", "lead")];
    let fact = full("lead:cl:working:%3:claude:Spec Model 9:high:");
    for status in [Status::Stopped, Status::Unknown] {
        let value = enrich(status, LEAD, &[Some(true)], Some(&fact), Some(&panes), NOW);
        assert_eq!(live(agent(&value, "lead")), unknown(), "{status:?}");
    }
}

// ---------------------------------------------------------------------------
// seam: account
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    super::cli::OwnedScratch::root("lsn", tag).keep()
}

fn account(value: &Value, name: &str) -> Option<(String, String)> {
    match agent(value, name).get("account") {
        Some(Value::Null) => None,
        Some(object @ Value::Obj(_)) => {
            assert_eq!(keys(object), ["tool", "source"], "account key order");
            Some((
                object
                    .get_str("tool")
                    .unwrap_or_else(|| panic!("account.tool: {}", value.render()))
                    .to_owned(),
                object
                    .get_str("source")
                    .unwrap_or_else(|| panic!("account.source: {}", value.render()))
                    .to_owned(),
            ))
        }
        other => panic!(
            "account missing or mistyped: {other:?} in {}",
            value.render()
        ),
    }
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|why| panic!("{} canonicalizes: {why}", path.display()))
}

#[test]
fn a_recorded_claude_home_joins_its_canonical_claude_json() {
    let dir = scratch_dir("claude");
    let home = dir.join("home");
    fs::create_dir_all(&home).expect("home");
    let meta = format!("{LEAD}config_home.main={}\n", home.display());
    let value = enrich(Status::Running, &meta, &[Some(true)], None, None, NOW);
    let expected = canonical(&home).join(".claude.json");
    assert_eq!(
        account(&value, "lead"),
        Some(("claude".to_owned(), expected.display().to_string()))
    );

    // Implicit mode joins through the recorded base.
    let base = dir.join("base");
    fs::create_dir_all(&base).expect("base");
    let meta = format!(
        "{LEAD}config_home.main=implicit:{}\nconfig_home_base.main={}\n",
        home.display(),
        base.display()
    );
    let value = enrich(Status::Running, &meta, &[Some(true)], None, None, NOW);
    let expected = canonical(&base).join(".claude.json");
    assert_eq!(
        account(&value, "lead"),
        Some(("claude".to_owned(), expected.display().to_string()))
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_codex_seat_joins_only_with_a_recorded_conversation() {
    let dir = scratch_dir("codex");
    let home = dir.join("codex-home");
    fs::create_dir_all(&home).expect("home");
    let base = format!(
        "seat.main=lead\nprofile.main=cx\nagent_bin.main=codex\nconfig_home.main={}\n",
        home.display()
    );
    let value = enrich(Status::Running, &base, &[Some(true)], None, None, NOW);
    assert_eq!(
        account(&value, "lead"),
        None,
        "no conversation, no proven identity"
    );

    let meta = format!("{base}harness_session.main=019a0000-0000-7000-8000-000000000001\n");
    let value = enrich(Status::Running, &meta, &[Some(true)], None, None, NOW);
    let expected = canonical(&home).join("sessions");
    assert_eq!(
        account(&value, "lead"),
        Some(("codex".to_owned(), expected.display().to_string()))
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn unjoinable_tools_and_unrecorded_homes_are_null_never_guessed() {
    let dir = scratch_dir("nojoin");
    let home = dir.join("h");
    fs::create_dir_all(&home).expect("home");
    for binary in ["agy", "grok", "muse", "opencode", "gemini"] {
        let meta = format!(
            "seat.main=lead\nprofile.main=cl\nagent_bin.main={binary}\nconfig_home.main={}\n",
            home.display()
        );
        let value = enrich(Status::Running, &meta, &[Some(true)], None, None, NOW);
        assert_eq!(account(&value, "lead"), None, "{binary}");
    }
    // A claude seat with no recorded home (legacy meta).
    let value = enrich(Status::Running, LEAD, &[Some(true)], None, None, NOW);
    assert_eq!(account(&value, "lead"), None, "legacy claude");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn account_is_durable_for_every_status_and_independent_of_the_fact() {
    let dir = scratch_dir("durable");
    let home = dir.join("home");
    fs::create_dir_all(&home).expect("home");
    let meta = format!("{LEAD}config_home.main={}\n", home.display());
    let expected = Some((
        "claude".to_owned(),
        canonical(&home).join(".claude.json").display().to_string(),
    ));
    for status in [Status::Running, Status::Stopped, Status::Unknown] {
        let value = enrich(status, &meta, &[None], None, None, NOW);
        assert_eq!(account(&value, "lead"), expected, "{status:?}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_account_source_is_escaped_by_the_json_writer() {
    let dir = scratch_dir("quote");
    // The home itself need not exist: the join canonicalizes its longest
    // existing ancestor, and the quote must reach the document escaped.
    let home = dir.join("q\"x");
    let meta = format!("{LEAD}config_home.main={}\n", home.display());
    let value = enrich(Status::Running, &meta, &[Some(true)], None, None, NOW);
    let expected = canonical(&dir).join("q\"x").join(".claude.json");
    assert_eq!(
        account(&value, "lead"),
        Some(("claude".to_owned(), expected.display().to_string()))
    );
    assert!(value.render().contains(r#"q\"x"#), "{}", value.render());
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// seam: coverage
// ---------------------------------------------------------------------------

#[test]
fn coverage_counts_live_seats_and_keeps_unknown_liveness_apart() {
    let dir = scratch_dir("cover");
    let home = dir.join("home");
    fs::create_dir_all(&home).expect("home");
    // Four seats: joined+live, unjoined+live, joined+unknown, joined+dead.
    let meta = format!(
        "seat.main=a\nprofile.main=cl\nagent_bin.main=claude\nconfig_home.main={h}\n\
         seat.worker.0=b\nprofile.worker.0=ag\nagent_bin.worker.0=agy\n\
         seat.worker.1=c\nprofile.worker.1=cl\nagent_bin.worker.1=claude\nconfig_home.worker.1={h}\n\
         seat.spawned.1=d\nprofile.spawned.1=cl\nagent_bin.spawned.1=claude\nconfig_home.spawned.1={h}\n",
        h = home.display()
    );
    let alive = [Some(true), Some(true), None, Some(false)];
    let value = enrich(Status::Running, &meta, &alive, None, None, NOW);
    assert_eq!(coverage(&value), Some((1, 2, 1)), "{}", value.render());

    // The fact never moves coverage.
    let panes = [pane("%3", "main", "a")];
    let with_fact = enrich(
        Status::Running,
        &meta,
        &alive,
        Some(&full("a:cl:working:%3:claude:Spec Model 9:high:")),
        Some(&panes),
        NOW,
    );
    assert_eq!(coverage(&with_fact), Some((1, 2, 1)));

    // Stopped: nothing is live. Unknown status: not established.
    let stopped = enrich(Status::Stopped, &meta, &alive, None, None, NOW);
    assert_eq!(coverage(&stopped), Some((0, 0, 0)));
    let unknown_status = enrich(Status::Unknown, &meta, &alive, None, None, NOW);
    assert_eq!(coverage(&unknown_status), None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_running_roster_covers_zero_of_zero() {
    let mut entry = SessionEntry::new("spec", Status::Running);
    ae::session::enrich_list_json(&mut entry, &[], None, None, Timestamp::from_epoch(NOW));
    assert_eq!(coverage(&entry.to_json()), Some((0, 0, 0)));
}

// ---------------------------------------------------------------------------
// end to end: `ae list --all --json` on a private server
// ---------------------------------------------------------------------------

fn tmux(socket: &Path, scratch: &Path, words: &[&str]) -> (bool, String) {
    let mut args = vec!["-S".to_owned(), socket.display().to_string()];
    args.extend(words.iter().map(|word| (*word).to_owned()));
    run_tmux(&args, scratch)
}

struct Server {
    socket: PathBuf,
    scratch: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let out = self.scratch.join("cleanup-out");
        let err = self.scratch.join("cleanup-err");
        let invocation = Invocation::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server");
        let _ = raw::run(&invocation, &self.scratch, &out, &err);
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

/// A running session `live` on a private socket: one pane running a non-shell
/// process, stamped as roster seat `main` = `lead`, its frame fact `busy`, and
/// a recorded claude home. Returns the server, the state root, the claude home
/// and the pane id.
fn live_session(tag: &str) -> (Server, PathBuf, PathBuf, String) {
    let scratch = scratch_dir(tag);
    assert!(
        tmux_present(&scratch),
        "tmux is not runnable here; the end-to-end arms need it"
    );
    let socket = scratch.join("s");
    let server = Server {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let home = scratch.join("claude-home");
    assert!(fs::create_dir_all(&home).is_ok(), "claude home");
    let meta_dir = root.join("sessions").join("live");
    assert!(fs::create_dir_all(&meta_dir).is_ok(), "meta dir");
    let written = fs::write(
        meta_dir.join("meta"),
        format!(
            "mode=local\nsession=live\ntmux_server_kind=socket\ntmux_server={}\n\
             seat.main=lead\nprofile.main=cl\nagent_bin.main=claude\nconfig_home.main={}\n",
            socket.display(),
            home.display()
        ),
    );
    assert!(written.is_ok(), "the record");
    // `exec`: tmux runs the pane command as `$SHELL -c <cmd>`, and dash (SHELL=/bin/sh
    // on Linux) keeps that wrapper in the foreground, so the pane would read as a
    // shell, which liveness counts as not alive (tests/it/transport.rs does the same).
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "live", "exec sleep 300"]
        )
        .0,
        "the session"
    );
    let running = (0..200).any(|_| {
        let (ok, out) = tmux(
            &socket,
            &scratch,
            &[
                "display-message",
                "-p",
                "-t",
                "live",
                "#{pane_dead}|#{pane_current_command}",
            ],
        );
        let done = ok && out.trim() == "0|sleep";
        if !done {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        done
    });
    assert!(running, "the pane never reported a live `sleep` foreground");
    for (option, value) in [
        ("@ae_agent", "lead"),
        ("@ae_slot", "main"),
        ("@ae_observed", "busy"),
    ] {
        assert!(
            tmux(
                &socket,
                &scratch,
                &["set-option", "-p", "-t", "live", option, value]
            )
            .0,
            "{option}"
        );
    }
    for (name, value) in [
        ("AE_HOME", root.display().to_string()),
        ("AE_SESSION", "1".to_owned()),
    ] {
        assert!(
            tmux(
                &socket,
                &scratch,
                &["set-environment", "-t", "live", name, &value]
            )
            .0,
            "{name}"
        );
    }
    let (ok, pane) = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-t", "live", "#{pane_id}"],
    );
    assert!(ok, "the pane id");
    (server, root, home, pane.trim().to_owned())
}

fn set_fact(server: &Server, fact: &str) {
    assert!(
        tmux(
            &server.socket,
            &server.scratch,
            &["set-option", "-t", "live", "@ae_agents", fact]
        )
        .0,
        "the hand-set roster fact"
    );
}

fn list(server: &Server, root: &Path, json: bool) -> String {
    let mut runner = super::cli::ae();
    runner.arg("list").arg("--all");
    if json {
        runner.arg("--json");
    }
    let output = runner
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("HOME", &server.scratch)
        .env("AE_HOME", root)
        .env("CONFIG_FILE", root.join("config"))
        .env("AE_NO_AUTOSTART", "1")
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &server.socket)
        .output()
        .unwrap_or_else(|why| panic!("ae list runs: {why}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

fn listed_session(stdout: &str) -> Value {
    let document = ae::json::parse(stdout.trim_end())
        .unwrap_or_else(|why| panic!("the digest parses: {why:?}\n{stdout}"));
    let Some(Value::Arr(sessions)) = document.get("sessions") else {
        panic!("sessions[] missing: {stdout}");
    };
    sessions
        .iter()
        .find(|session| session.get_str("name") == Some("live"))
        .cloned()
        .unwrap_or_else(|| panic!("session live missing: {stdout}"))
}

#[test]
fn ae_list_json_reports_a_live_seat_from_its_fresh_fact_and_recorded_home() {
    let (server, root, home, pane) = live_session("e2e");
    let now = Timestamp::now().epoch();
    set_fact(
        &server,
        &fact_at(
            now,
            60,
            &format!("lead:cl:working:{pane}:claude:Spec Model 9:xhigh:"),
        ),
    );
    let stdout = list(&server, &root, true);
    let session = listed_session(&stdout);
    assert_eq!(session.get_str("status"), Some("running"), "{stdout}");
    let lead = agent(&session, "lead");
    assert_eq!(
        lead.get_str("observed"),
        Some("busy"),
        "activity unchanged: {stdout}"
    );
    assert_eq!(
        live(lead),
        triple("claude", "Spec Model 9", "xhigh"),
        "{stdout}"
    );
    assert_eq!(
        account(&session, "lead"),
        Some((
            "claude".to_owned(),
            canonical(&home).join(".claude.json").display().to_string()
        )),
        "{stdout}"
    );
    assert_eq!(coverage(&session), Some((1, 1, 0)), "{stdout}");

    // The human table is not where these facts go.
    let human = list(&server, &root, false);
    assert!(!human.contains("Spec Model 9"), "{human}");
}

#[test]
fn ae_list_json_reads_unknown_for_a_missing_or_stale_fact_and_keeps_the_rest() {
    let (server, root, home, pane) = live_session("stale");
    let expect_rest = |stdout: &str| {
        let session = listed_session(stdout);
        let lead = agent(&session, "lead");
        assert_eq!(lead.get_str("observed"), Some("busy"), "{stdout}");
        assert_eq!(live(lead), unknown(), "{stdout}");
        assert_eq!(
            account(&session, "lead"),
            Some((
                "claude".to_owned(),
                canonical(&home).join(".claude.json").display().to_string()
            )),
            "{stdout}"
        );
        assert_eq!(coverage(&session), Some((1, 1, 0)), "{stdout}");
    };
    expect_rest(&list(&server, &root, true));

    let old = Timestamp::now().epoch() - 1_000;
    set_fact(
        &server,
        &fact_at(
            old,
            60,
            &format!("lead:cl:working:{pane}:claude:Spec Model 9:xhigh:"),
        ),
    );
    expect_rest(&list(&server, &root, true));

    // A fresh fact naming another pane is not this seat.
    set_fact(
        &server,
        &fact_at(
            Timestamp::now().epoch(),
            60,
            "lead:cl:working:%999:claude:Spec Model 9:xhigh:",
        ),
    );
    expect_rest(&list(&server, &root, true));
}
