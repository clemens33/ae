//! #296 follow-up 4. Oracle: lead ruling 2026-11-17: an automatic chain
//! returns to its original profile@client, or refuses once without moving.
//! A real spawn and departure mint the record under test. Only the explicitly
//! legacy case installs historical bare records. The done ref is the durable
//! spelling: profile@client for a labeled source, the existing bare profile otherwise.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent acceptance owns private tools, provider stores and journals"
)]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use ae::autoreseat::{ATTEMPT_ACTION, DONE_ACTION, REFUSED_ACTION, RETURN_ACTION};
use ae::events::Event;
use ae::time::Timestamp;

use super::clientfollow_resume_spec::{Rig, Shape, success};

const SLOT: &str = "spawned.0";

fn configured(tag: &str) -> Rig {
    let rig = Rig::new(tag, Shape::Flagless);
    for store in ["store-c", "store-d"] {
        std::fs::create_dir(rig.scratch.join(store)).expect("independent substitute account");
    }
    let script = rig.scratch.join("harness.pl");
    let identity = format!(
        "{}\n[clients]\nsub = {} config_home={}\nnext = {} config_home={}\n\
         [profiles]\naway = \"sub {} --model chosen\"\n\
         away2 = \"next {} --model chosen\"\nrootaccount = \"carry {} --model chosen\"\n",
        rig.identity("b", "store-b", "claude"),
        rig.binary("c", "claude").display(),
        rig.scratch.join("store-c").display(),
        rig.binary("a", "claude").display(),
        rig.scratch.join("store-d").display(),
        script.display(),
        script.display(),
        script.display(),
    );
    rig.write_config(
        &identity,
        "[workspace]\nauto_reseat_at = 95\nauto_reseat_grace_secs = 0\n\
         [auto_reseat]\np = away\naway = away2\n",
        true,
    );
    rig.launch();
    rig
}

fn sample(rig: &Rig, store: &str, used: u8) {
    let now = Timestamp::now().epoch();
    std::fs::write(
        rig.scratch.join(store).join(".claude.json"),
        format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":{used},\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}\n",
            now * 1_000,
            Timestamp::from_epoch(now + 86_400),
        ),
    )
    .expect("provider quota evidence");
}

fn records(rig: &Rig, action: &str) -> Vec<Event> {
    std::fs::read_to_string(rig.dir().join("events.jsonl"))
        .expect("real automatic journal")
        .lines()
        .map(|line| Event::parse_line(line).expect("complete fixture or product record"))
        .filter(|event| {
            event.action == action
                && ae::watchdog::event_is_addressed_to(event, "cfsession", SLOT, "helper")
        })
        .collect()
}

fn append(rig: &Rig, action: &str, at: i64, reference: &str, summary: &str) {
    let path = rig.dir().join("events.jsonl");
    let mut text = std::fs::read_to_string(&path).expect("existing journal");
    writeln!(
        text,
        "{{\"ts\":\"{}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"helper\",\"target_slot\":\"{SLOT}\",\"target_session\":\"cfsession\",\"ref\":\"{reference}\",\"summary\":\"{summary}\"}}",
        Timestamp::from_epoch(at),
    )
    .expect("fixed ASCII journal text");
    std::fs::write(path, text).expect("watchdog booking");
}

fn leg(rig: &Rig, key: Timestamp) -> std::process::Output {
    rig.command()
        .arg("_auto-reseat")
        .arg(rig.dir())
        .args([SLOT, &key.to_string()])
        .output()
        .expect("real automatic reseat leg")
}

fn park(rig: &Rig) {
    let pane = rig.pane(SLOT).expect("current substitute pane");
    assert!(
        rig.tmux(&["respawn-pane", "-k", "-t", &pane, "exec /bin/sh"])
            .0
    );
    super::support::pane_command(
        |args| rig.tmux(args),
        &pane,
        Instant::now() + super::support::PANE_WAIT,
        super::support::is_shell,
    );
}

fn depart(rig: &Rig, from_store: &str, destination: &str, second: bool) {
    sample(rig, from_store, 98);
    sample(rig, "store-c", if second { 98 } else { 10 });
    sample(rig, "store-d", 10);
    rig.provider_history(from_store);
    park(rig);
    let at = Timestamp::now().epoch() - if second { 10 } else { 20 };
    let key = Timestamp::from_epoch(at);
    let before = records(rig, DONE_ACTION).len();
    append(
        rig,
        "auto-reseat-headroom",
        at,
        "",
        "headroom 98% weekly_all 7d",
    );
    append(
        rig,
        ATTEMPT_ACTION,
        at + 1,
        &key.to_string(),
        "automatic departure",
    );
    success(&leg(rig, key));
    assert_eq!(rig.row("profile.spawned.0").as_deref(), Some(destination));
    assert_eq!(rig.row("client.spawned.0"), None);
    assert_eq!(
        records(rig, DONE_ACTION).len(),
        before + 1,
        "the real departure completed"
    );
    rig.wait_exec(
        if second { "a" } else { "c" },
        if second { "store-d" } else { "store-c" },
    );
}

fn departed(tag: &str, pair: bool) -> Rig {
    let rig = configured(tag);
    success(&rig.spawn("helper", if pair { "p@carry" } else { "p" }));
    rig.wait_exec(
        if pair { "b" } else { "a" },
        if pair { "store-b" } else { "store-a" },
    );
    sample(&rig, "store-a", 20);
    sample(&rig, "store-b", 20);
    depart(
        &rig,
        if pair { "store-b" } else { "store-a" },
        "away",
        false,
    );
    rig
}

fn recover(rig: &Rig) {
    let done = records(rig, DONE_ACTION)
        .last()
        .expect("completed departure")
        .ts
        .epoch();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Timestamp::now().epoch() <= done {
        assert!(Instant::now() < deadline, "fresh sample clock advanced");
        std::thread::sleep(Duration::from_millis(20));
    }
    for store in ["store-a", "store-b", "store-c", "store-d"] {
        sample(rig, store, 20);
    }
}

fn book_return(rig: &Rig) -> Timestamp {
    let at = Timestamp::now().epoch();
    let key = Timestamp::from_epoch(at);
    append(
        rig,
        RETURN_ACTION,
        at,
        "",
        "return after fresh origin relief",
    );
    append(
        rig,
        ATTEMPT_ACTION,
        at + 1,
        &key.to_string(),
        "automatic return",
    );
    key
}

fn returned(rig: &Rig, pair: bool) {
    let current = if rig.row("profile.spawned.0").as_deref() == Some("away2") {
        "store-d"
    } else {
        "store-c"
    };
    rig.provider_history(current);
    park(rig);
    std::fs::write(&rig.receipt, "").expect("return-only executable receipts");
    let before = records(rig, DONE_ACTION).len();
    let key = book_return(rig);
    success(&leg(rig, key));
    assert_eq!(rig.row("profile.spawned.0").as_deref(), Some("p"));
    assert_eq!(
        rig.row("client.spawned.0").as_deref(),
        pair.then_some("carry")
    );
    rig.wait_exec(
        if pair { "b" } else { "a" },
        if pair { "store-b" } else { "store-a" },
    );
    assert_eq!(records(rig, DONE_ACTION).len(), before + 1);
    assert!(records(rig, REFUSED_ACTION).is_empty());
}

#[test]
fn labeled_root_is_persisted_as_a_pair_and_returns_to_its_account() {
    let rig = departed("pin-return-pair", true);
    assert_eq!(
        records(&rig, DONE_ACTION)[0].reference.as_deref(),
        Some("p@carry")
    );
    recover(&rig);
    returned(&rig, true);
    let receipt = rig.execs();
    assert!(
        !receipt.contains(&format!(
            "ACCOUNT={} ",
            std::fs::canonicalize(rig.scratch.join("store-a"))
                .expect("canonical default account")
                .display()
        )),
        "default account never runs: {receipt}"
    );
}

#[test]
fn a_labeled_chain_returns_to_the_root_after_two_automatic_hops() {
    let rig = departed("pin-return-two", true);
    depart(&rig, "store-c", "away2", true);
    recover(&rig);
    returned(&rig, true);
}

#[derive(Clone, Copy)]
enum Broken {
    Unapproved,
    MissingClient,
    OtherAdapter,
    UnprovableAccount,
}

fn substitute_pid(rig: &Rig) -> u32 {
    let pane = rig.pane(SLOT).expect("live substitute pane");
    let (ok, root) = rig.tmux(&["display-message", "-p", "-t", &pane, "#{pane_pid}"]);
    assert!(ok, "substitute pane identity reads");
    let root: u32 = root.trim().parse().expect("substitute shell pid");
    ae::procs::snapshot()
        .expect("real process table")
        .iter()
        .find(|proc| proc.ppid == root && proc.comm.rsplit('/').next() == Some("claude"))
        .expect("the actual substitute remains running")
        .pid
}

fn broken_root(kind: Broken) {
    let rig = departed("pin-return-refuse", true);
    recover(&rig);
    let config = std::fs::read_to_string(&rig.config).expect("global config");
    let carry = config
        .lines()
        .find(|line| line.starts_with("carry = "))
        .expect("approved root client");
    let (changed, reason) = match kind {
        Broken::Unapproved => (config.replace("p = carry\n", ""), "approved"),
        Broken::MissingClient => (config.replace(carry, ""), "carry"),
        Broken::OtherAdapter => (
            config.replace(
                carry,
                &carry.replace(
                    &rig.binary("b", "claude").display().to_string(),
                    &rig.binary("b", "codex").display().to_string(),
                ),
            ),
            "codex",
        ),
        Broken::UnprovableAccount => {
            let changed = config.replace(
                carry,
                &format!("carry = {}", rig.binary("b", "claude").display()),
            );
            (
                changed.replace(
                    "p = \"default ",
                    &format!(
                        "p = \"CLAUDE_CONFIG_DIR=relative/store {} ",
                        rig.binary("a", "claude").display()
                    ),
                ),
                "account",
            )
        }
    };
    assert_ne!(
        changed, config,
        "the root pair's authority actually changes"
    );
    ae::config::parse_identity(&changed).expect("the refused root remains valid config grammar");
    std::fs::write(&rig.config, changed).expect("root pair no longer approved or provable");
    let before_meta = rig.meta();
    let before_execs = rig.execs();
    let before_pid = substitute_pid(&rig);
    let before_stop = records(&rig, "stop").len();
    let before_done = records(&rig, DONE_ACTION).len();
    let key = book_return(&rig);
    let _ = leg(&rig, key);
    assert_eq!(
        rig.meta(),
        before_meta,
        "refusal leaves the substitute identity"
    );
    assert_eq!(
        rig.execs(),
        before_execs,
        "refusal starts no binary on any account"
    );
    assert_eq!(records(&rig, DONE_ACTION).len(), before_done);
    assert_eq!(
        substitute_pid(&rig),
        before_pid,
        "refusal keeps the substitute process"
    );
    assert_eq!(
        records(&rig, "stop").len(),
        before_stop,
        "refusal precedes any stop"
    );
    let refused = records(&rig, REFUSED_ACTION);
    assert_eq!(
        refused.len(),
        1,
        "one named refusal, not a bare-profile fallback"
    );
    let summary = refused[0]
        .summary
        .as_deref()
        .expect("refusal names a reason");
    assert!(summary.contains(reason), "expected {reason}: {summary}");
    let _ = leg(&rig, key);
    assert_eq!(
        records(&rig, REFUSED_ACTION).len(),
        1,
        "repeating one closed return does not duplicate refusal"
    );
    assert_eq!(rig.meta(), before_meta);
    assert_eq!(rig.execs(), before_execs);
    assert_eq!(substitute_pid(&rig), before_pid);
}

#[test]
fn unapproved_root_pair_refuses_once_without_a_default_account_fallback() {
    broken_root(Broken::Unapproved);
}

#[test]
fn missing_root_client_refuses_once_without_moving_the_substitute() {
    broken_root(Broken::MissingClient);
}

#[test]
fn root_pair_on_another_adapter_refuses_once_without_moving_the_substitute() {
    broken_root(Broken::OtherAdapter);
}

#[test]
fn unprovable_root_account_refuses_once_without_moving_the_substitute() {
    broken_root(Broken::UnprovableAccount);
}

#[test]
fn bare_root_keeps_the_existing_bare_record_and_return() {
    let rig = departed("pin-return-bare", false);
    assert_eq!(
        records(&rig, DONE_ACTION)[0].reference.as_deref(),
        Some("p")
    );
    recover(&rig);
    returned(&rig, false);
}

#[test]
fn an_old_bare_departure_record_keeps_its_default_account_return() {
    let rig = departed("pin-return-legacy", true);
    // Older ae emitted bare refs even for a departing labeled seat. Preserve
    // every actual event, replacing only the done ref with its old spelling.
    let path = rig.dir().join("events.jsonl");
    let prior = std::fs::read_to_string(&path).expect("real departure history");
    let old = prior.replace("\"ref\":\"p@carry\"", "\"ref\":\"p\"");
    std::fs::write(path, old).expect("historical bare-ref compatibility");
    assert_eq!(
        records(&rig, DONE_ACTION)[0].reference.as_deref(),
        Some("p"),
        "the historical spelling was installed"
    );
    recover(&rig);
    returned(&rig, false);
}
