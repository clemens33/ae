//! #250 O3 phase 3 acceptance, authored independently from the owner ruling.
//! R1/R2/R3/R4/R5/R9/R10/R11/R12. Physical private tmux, real transport,
//! ordinary console admission and journal bytes; no relay/receipt substitute.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private state and inspects its effects"
)]

use super::o2guard_p2_spec::{Seat, World, ledger, uuid_of};
use ae::events::Event;
use ae::json::{self, Value};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

pub(crate) type Out = (Option<i32>, String, String);
pub(crate) struct Rig {
    // Owner owns server/root; World is a borrowed helper view.
    pub(crate) owner: super::app_writer_spec::Rig,
    pub(crate) world: std::mem::ManuallyDrop<World>,
    pub(crate) caller: Seat,
}
impl Rig {
    pub(crate) fn new(tag: &str) -> Self {
        let owner = super::app_writer_spec::Rig::new(tag);
        let world = World {
            root: owner.root.clone(),
            sock: owner.socket.clone(),
        };
        let caller = world.session("orch", "boss", "meta_agent=true\n");
        Self {
            owner,
            world: std::mem::ManuallyDrop::new(world),
            caller,
        }
    }
    pub(crate) fn run(&self, tail: &[&str]) -> Out {
        let dir = self.caller.dir.to_string_lossy();
        let argv = [&[ae::cli::RELAY, &dir][..], tail].concat();
        self.world.run(
            Some(&self.caller.pane),
            &argv,
            &[("AE_SEND_DEFER_SEC", "1")],
        )
    }
    pub(crate) fn relay(&self, body: &str) -> Out {
        self.run(&[&self.owner.home, body])
    }
    pub(crate) fn asks(&self) -> Vec<Event> {
        events(&self.owner.tool.dir)
            .into_iter()
            .filter(|e| e.action == "ask")
            .collect()
    }
    pub(crate) fn ask(&self, body: &str) -> String {
        let n = self.asks().len();
        let out = self.relay(body);
        assert_eq!(out.0, Some(0), "relay: {}", words(&out));
        let asks = self.asks();
        assert_eq!(
            asks.len(),
            n + 1,
            "one new target-local request: {}",
            words(&out)
        );
        let id = asks[n].reference.clone().expect("request id");
        assert!(
            !id.is_empty() && words(&out).contains(&id),
            "id returned: {}",
            words(&out)
        );
        receipt(&out, &id, "sent", "pending");
        id
    }
    pub(crate) fn chat(&self, session: &str) -> String {
        let out = self.world.run(None, &["chat", session], &[]);
        assert_eq!(out.0, Some(0), "chat read: {}", words(&out));
        words(&out)
    }
    pub(crate) fn keys(&self) -> Vec<u8> {
        fs::read(self.world.root.join("enters")).expect("Enter receipt")
    }
    pub(crate) fn sent(&self) -> String {
        fs::read_to_string(self.world.root.join("received")).expect("wire receipt")
    }
    pub(crate) fn reply(&self, id: &str, body: &str) {
        let dir = self.owner.tool.dir.to_string_lossy();
        let before = fs::read(&self.caller.receipt).expect("caller receipt");
        let out = self.world.run(
            Some(&self.owner.tool.pane),
            &[ae::cli::REPLY, &dir, id, body],
            &[],
        );
        assert_eq!(out.0, Some(0), "local reply: {}", words(&out));
        assert_eq!(
            fs::read(&self.caller.receipt).expect("caller receipt"),
            before,
            "reply pastes nothing to orchestrator"
        );
    }
    pub(crate) fn refused(&self, dead: bool) -> String {
        if dead {
            self.owner
                .tmux(&["respawn-pane", "-k", "-t", &self.owner.tool.pane, "exec sh"]);
        } else {
            self.owner.raw(&self.owner.tool.pane, "human unfinished");
            self.owner.wait(&self.owner.tool.pane, "human draft", |s| {
                s.contains("human unfinished")
            });
        }
        let keys = self.keys();
        let out = self.relay("must not be delivered");
        no_delivery(self, &out, &keys, 0);
        assert_eq!(out.0, Some(1));
        let id = self.attempt();
        receipt(&out, &id, "not-delivered", "absent");
        assert!(
            rows(&self.caller.dir)
                .iter()
                .any(|e| e.get_str("action") == Some("relay-result")
                    && e.get_str("outcome") == Some("not-delivered"))
        );
        if !dead {
            assert!(
                self.owner
                    .screen(&self.owner.tool.pane)
                    .contains("human unfinished")
            );
        }
        id
    }
    pub(crate) fn attempt(&self) -> String {
        let claims: Vec<_> = events(&self.caller.dir)
            .into_iter()
            .filter(|e| e.action == "relay-attempt")
            .collect();
        claims
            .last()
            .and_then(|e| e.reference.clone())
            .expect("durable pre-delivery attempt id")
    }
}
pub(crate) fn receipt(out: &Out, id: &str, outcome: &str, request: &str) {
    let expected = format!("{id}\noutcome: {outcome}\nrequest: {request}\n");
    assert!(
        out.1.starts_with(&expected),
        "stdout contract: {:?}; expected {:?}",
        out.1,
        expected
    );
}
pub(crate) fn words(out: &Out) -> String {
    format!("{}\n{}", out.1, out.2)
}
pub(crate) fn events(dir: &Path) -> Vec<Event> {
    ledger(dir)
        .lines()
        .map(|l| Event::parse_line(l).expect("valid journal"))
        .collect()
}
pub(crate) fn rows(dir: &Path) -> Vec<Value> {
    ledger(dir)
        .lines()
        .map(|l| json::parse(l).expect("JSON event"))
        .collect()
}
pub(crate) fn wait_for(why: &str, met: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(15);
    while !met() {
        assert!(Instant::now() < until, "fixture never reached {why}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
pub(crate) fn no_delivery(r: &Rig, out: &Out, keys: &[u8], asks: usize) {
    assert_ne!(out.0, Some(0), "refusal: {}", words(out));
    assert_eq!(r.keys(), keys, "no target Enter");
    assert_eq!(r.asks().len(), asks, "no pending ask created");
}
pub(crate) fn echo_count(r: &Rig, id: &str) -> usize {
    events(&r.caller.dir)
        .iter()
        .filter(|e| {
            e.action == "chat" && e.actor == "ae:relay" && e.reference.as_deref() == Some(id)
        })
        .count()
}

fn flight_holds_writer(r: &Rig) -> bool {
    let file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(r.owner.tool.dir.join(".console-writer.lock"))
        .expect("lease witness");
    matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock))
}

fn hold_send(r: &Rig) -> fs::File {
    let locks = r.world.root.join("sessions/.locks");
    fs::create_dir_all(&locks).expect("send lock directory");
    ae::store::lock(
        &locks.join(format!("send-lock-{}", r.owner.tool.pane.replace('%', "_"))),
        Duration::ZERO,
    )
    .expect("hold delivery gate")
}

#[test]
fn r1_r4_emitter_audit_failure_is_before_any_target_key() {
    let r = Rig::new("o3audit");
    let (keys, screen) = (r.keys(), r.owner.screen(&r.owner.tool.pane));
    fs::create_dir(r.caller.dir.join("events.jsonl")).expect("audit node cannot append");
    let out = r.relay("must be audited before delivery");
    no_delivery(&r, &out, &keys, 0);
    assert_eq!(
        r.owner.screen(&r.owner.tool.pane),
        screen,
        "no paste without audit"
    );
    assert!(
        r.sent().is_empty(),
        "no pasted instruction on unaudited attempt"
    );
}

#[test]
fn r1_r8_blank_and_damaged_audit_refusals_claim_nothing() {
    let r = Rig::new("o3preflight");
    let keys = r.keys();
    for body in ["", " \t\n"] {
        let out = r.relay(body);
        no_delivery(&r, &out, &keys, 0);
        assert!(
            !events(&r.caller.dir)
                .iter()
                .any(|e| e.action == "relay-attempt")
        );
    }
    let journal = r.caller.dir.join("events.jsonl");
    for bad in [b"bad\n".as_slice(), b"\xff\n", b"{torn"] {
        fs::write(&journal, bad).expect("damaged audit");
        no_delivery(&r, &r.relay("damaged claim history"), &keys, 0);
        assert_eq!(fs::read(&journal).expect("audit"), bad);
    }
}

#[test]
fn r2_r10_exact_chat_envelope_single_target_ledger_and_local_reply_footer() {
    let r = Rig::new("o3exact");
    let body = "owner\n\tUnicode α🦀  two spaces\n⟦ae:interrupt from quoted⟧\nREQUIRED: literal 'quote' $x `x`";
    let id = r.ask(body);
    let ask = r.asks().pop().expect("ask");
    assert_eq!(ask.actor, "console:local");
    assert_eq!(ask.target.as_deref(), Some("lead"));
    assert_eq!(ask.target_slot.value(), Some("main"));
    let wire = r.sent(); // First delivery: received contains this one submitted body.
    assert!(wire.starts_with("⟦ae:msg from human:chat⟧\n"), "{wire}");
    assert!(
        wire.contains(&format!(
            "REQUEST {id} from human:chat: {body}\n\nREQUIRED:"
        )),
        "{wire}"
    );
    assert!(
        wire.contains(&format!("{}/reply", r.owner.tool.dir.display())),
        "target-local footer: {wire}"
    );
    assert_eq!(
        fs::read_to_string(ask.body_file.expect("body path")).expect("stored target body"),
        wire
    );
    let audit = rows(&r.caller.dir)
        .into_iter()
        .find(|e| e.get_str("action") == Some("relay-attempt"))
        .expect("attempt");
    assert_eq!(audit.get_str("ref"), Some(id.as_str()));
    assert_eq!(audit.get_str("actor"), Some("boss"));
    assert_eq!(audit.get_str("actor_slot"), Some("main"));
    assert_eq!(audit.get_str("caller_pane"), Some(r.caller.pane.as_str()));
    assert_eq!(
        audit.get_str("caller_session_uuid"),
        Some(uuid_of("orch").as_str())
    );
    assert_eq!(
        fs::read_to_string(audit.get_str("body_file").expect("audit body")).expect("exact audit"),
        body
    );
    let raw = rows(&r.owner.tool.dir)
        .into_iter()
        .find(|e| e.get_str("action") == Some("ask"))
        .expect("ask JSON");
    assert_eq!(raw.get_str("relay_origin"), Some(uuid_of("orch").as_str()));
    assert!(
        !events(&r.caller.dir)
            .iter()
            .any(|e| ["ask", "review"].contains(&e.action.as_str())),
        "no second pending copy"
    );
    assert_eq!(echo_count(&r, &id), 1);
    let target_view = r.chat(&r.owner.home);
    assert!(
        target_view.contains("via orchestrator orch"),
        "{target_view}"
    );
    assert!(
        !target_view.contains(&uuid_of("orch")),
        "UUID is record detail, not badge"
    );
    assert!(
        r.chat("orch").contains(&id),
        "emitter lane echo is correlated"
    );
}

// Hold the real target journal append, observe the real submitted instruction,
// then kill only our child. This is the documented post-key/pre-opening gap,
// not an invented pending reservation or a mocked transport.
#[test]
fn r4_crash_after_delivery_keeps_emitter_claim_without_fabricated_target_opening() {
    let r = Rig::new("o3crash");
    let lock = ae::store::lock(
        &ae::store::lock_path(&r.owner.tool.dir.join("events.jsonl")),
        Duration::ZERO,
    )
    .expect("hold target opening append");
    let mut cmd = super::cli::ae();
    cmd.env("AE_HOME", &r.world.root)
        .env("TMUX", format!("{},0,0", r.world.sock.display()))
        .env("TMUX_PANE", &r.caller.pane)
        .args([
            ae::cli::RELAY,
            &r.caller.dir.to_string_lossy(),
            &r.owner.home,
            "crash-cut instruction",
        ]);
    let mut child = cmd.spawn().expect("relay child");
    wait_for("real post-delivery cut", || {
        r.sent().contains("crash-cut instruction")
    });
    assert!(!r.attempt().is_empty(), "audit was durable before keys");
    assert!(r.asks().is_empty(), "no pre-send target reservation");
    child.kill().expect("kill only owned relay child");
    child.wait().expect("reap owned child");
    drop(lock);
    assert!(r.asks().is_empty());
    assert_eq!(
        events(&r.caller.dir)
            .iter()
            .filter(|e| e.action == "relay-attempt")
            .count(),
        1
    );
}

#[test]
fn r5_verified_receipt_survives_opening_append_failure_with_request_gap() {
    for unreadable in [true, false] {
        let r = Rig::new("o3recordfail");
        let journal = r.owner.tool.dir.join("events.jsonl");
        let lock = ae::store::lock(&ae::store::lock_path(&journal), Duration::ZERO)
            .expect("hold target opening append until timeout");
        let body = "known submitted but opening cannot append";
        let out = std::thread::scope(|scope| {
            let relay = scope.spawn(|| r.relay(body));
            if unreadable {
                wait_for("real dispatch before read gap", || r.sent().contains(body));
                fs::remove_file(&journal).expect("remove owned target journal");
                fs::create_dir(&journal).expect("make post-dispatch journal unreadable");
            }
            relay.join().expect("relay finished")
        });
        drop(lock);
        assert_eq!(
            out.0,
            Some(0),
            "known Verified remains Sent: {}",
            words(&out)
        );
        let id = out.1.lines().next().unwrap_or_default();
        receipt(
            &out,
            id,
            "sent",
            if unreadable { "unknown" } else { "absent" },
        );
        assert_eq!(id, r.attempt(), "receipt id keys the durable attempt");
        assert!(r.sent().contains(body));
        if unreadable {
            assert!(out.2.contains("read gap"), "{}", out.2);
            assert!(out.2.contains(id), "{}", out.2);
            fs::remove_dir(&journal).expect("remove owned unreadable sentinel");
        }
        assert!(r.asks().is_empty(), "no invented pending ask");
    }
}

#[test]
fn r5_busy_draft_and_dead_pane_prove_not_delivered() {
    for dead in [false, true] {
        Rig::new(if dead { "o3dead" } else { "o3busy" }).refused(dead);
    }
}

#[test]
fn r8_final_envelope_cap_refuses_even_when_body_alone_fits() {
    let r = Rig::new("o3capbytes");
    let body = "x".repeat(8_192);
    let keys = r.keys();
    let out = r.relay(&body);
    no_delivery(&r, &out, &keys, 0);
    assert!(
        words(&out).contains("8192"),
        "names the byte bound: {}",
        words(&out)
    );
    assert!(r.sent().is_empty(), "no file-pointer substitute");
    assert!(
        !events(&r.caller.dir)
            .iter()
            .any(|e| e.action == "relay-attempt"),
        "size preflight precedes claim"
    );
}

#[test]
fn r9_writer_and_admission_locks_refuse_without_waiting_or_draft_changes() {
    let r = Rig::new("o3locks");
    let draft = r.owner.tool.dir.join("console.draft");
    fs::write(&draft, "kept owner draft").expect("draft sentinel");
    for name in [".console-writer.lock", ".console-admission.lock"] {
        let lock = ae::store::lock(&r.owner.tool.dir.join(name), Duration::ZERO)
            .expect("held kernel lock");
        let keys = r.keys();
        let start = Instant::now();
        let out = r.relay("cannot take held lock");
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "relay waited on {name}"
        );
        no_delivery(&r, &out, &keys, 0);
        assert_eq!(
            fs::read_to_string(&draft).expect("draft"),
            "kept owner draft"
        );
        drop(lock);
    }
    let journal = r.owner.tool.dir.join("events.jsonl");
    fs::write(&journal, "bad\n").expect("damaged target ledger");
    let keys = r.keys();
    let out = r.relay("unreadable admission proves no request state");
    receipt(&out, &r.attempt(), "not-delivered", "unknown");
    assert_eq!(out.0, Some(1));
    assert_eq!(r.keys(), keys);
    assert!(out.2.contains("read gap") && out.2.contains(&r.attempt()));
    assert_eq!(fs::read_to_string(&journal).expect("journal"), "bad\n");
    fs::write(&journal, "").expect("restore owned journal");
    r.owner
        .meta("0199c0de-aaaa-4890-abcd-ef0123456789", "colead");
    let keys = r.keys();
    let out = r.relay("replaced target incarnation");
    no_delivery(&r, &out, &keys, 0);
    receipt(&out, &r.attempt(), "not-delivered", "absent");
}

#[test]
fn r9_r12_concurrent_owner_flight_blocks_relay_and_shared_cap_cannot_overrun() {
    let r = Rig::new("o3appflight");
    let pane = r.owner.console(false);
    r.owner.raw(&pane, "i");
    r.owner
        .wait(&pane, "owner composing", crate::app_mode::writing);
    for n in 1..=3 {
        let body = format!("owner {n}");
        r.owner.typed(&pane, &body);
        r.owner.submit(&pane, &body, n);
    }
    r.owner.typed(&pane, "real app flight");
    let send = hold_send(&r);
    r.owner.key(&pane, "Enter");
    wait_for("real app flight writer lease", || flight_holds_writer(&r));
    let keys = r.keys();
    let start = Instant::now();
    let out = r.relay("relay cannot bypass app flight");
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "zero-wait lease refusal"
    );
    assert!(
        words(&out).contains("another window is sending") || words(&out).contains("held at"),
        "writer refusal: {}",
        words(&out)
    );
    no_delivery(&r, &out, &keys, 3);
    drop(send);
    r.owner.wait(&pane, "app completes original flight", |s| {
        s.contains("sent") && r.asks().len() == 4
    });
    r.owner.free();
    let keys = r.keys();
    no_delivery(&r, &r.relay("four owner asks exhaust relay cap"), &keys, 4);
    r.owner.typed(&pane, "owner fifth");
    r.owner.submit(&pane, "owner fifth", 5);
}

#[test]
fn r11_main_is_default_and_worker_refusal_names_pair() {
    let r = Rig::new("o3targets");
    r.ask("bare chooses main");
    assert_eq!(r.asks()[0].target_slot.value(), Some("main"));
    let meta = fs::read_to_string(r.owner.tool.dir.join("meta")).expect("meta");
    fs::write(
        r.owner.tool.dir.join("meta"),
        meta + "seat.spawned.1=scout\nagent_bin.spawned.1=gemini\n",
    )
    .expect("worker roster");
    let keys = r.keys();
    let target = format!("{}:scout", r.owner.home);
    let out = r.run(&[&target, "unsupported worker relay"]);
    no_delivery(&r, &out, &keys, 1);
    for word in ["lead-pair", "lead", "colead"] {
        assert!(words(&out).contains(word), "{word}: {}", words(&out));
    }
    let out = r.run(&[&format!("{}:lead", r.owner.home), "explicit main"]);
    assert_eq!(out.0, Some(0), "{}", words(&out));
    assert_eq!(r.asks().len(), 2);
    let file = r.world.root.join("peer-receipt");
    fs::write(&file, "").expect("peer receipt");
    let command = format!(
        "printf '> '; stty raw -echo; exec cat -u > {}",
        file.display()
    );
    let (ok, pane) = r.world.tmux(
        "new-window -d -P -F #{pane_id} -t",
        &[&r.owner.home, &command],
    );
    assert!(ok);
    let pane = pane.trim();
    r.world.stamp(pane, "worker.0", "colead");
    let meta = fs::read_to_string(r.owner.tool.dir.join("meta")).expect("meta");
    fs::write(
        r.owner.tool.dir.join("meta"),
        meta.replace("agent_bin.worker.0=codex", "agent_bin.worker.0=gemini"),
    )
    .expect("unmodelled peer");
    let out = r.run(&[&format!("{}:colead", r.owner.home), "explicit pair peer"]);
    assert_eq!(out.0, Some(1), "unmodelled peer receipt is Uncertain");
    receipt(&out, &r.attempt(), "uncertain", "pending");
    wait_for("peer bytes", || {
        fs::read_to_string(&file)
            .expect("peer receipt")
            .contains("explicit pair peer")
    });
    assert_eq!(r.asks().len(), 3);
    assert_eq!(r.asks()[2].target_slot.value(), Some("worker.0"));
}

#[test]
fn r12_four_relays_leave_owner_fifth_slot_and_refusal_names_open_ids() {
    let r = Rig::new("o3caprelay");
    let ids: Vec<_> = (0..4).map(|_| r.ask("repeat deliberately")).collect();
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        4
    );
    let keys = r.keys();
    let out = r.relay("fifth relay refused");
    no_delivery(&r, &out, &keys, 4);
    assert!(words(&out).contains("cap-full"), "{}", words(&out));
    for id in &ids {
        assert!(words(&out).contains(id), "missing open id {id}");
    }
    let pane = r.owner.console(false);
    r.owner.raw(&pane, "i");
    r.owner
        .wait(&pane, "owner composes", crate::app_mode::writing);
    r.owner.typed(&pane, "reserved owner fifth");
    r.owner.submit(&pane, "reserved owner fifth", 5);
    assert_eq!(r.asks().len(), 5, "relay leaves owner fifth slot");
}

#[test]
fn r12_owner_asks_consume_relay_capacity_and_close_frees_shared_count() {
    let r = Rig::new("o3capowner");
    let pane = r.owner.console(false);
    r.owner.raw(&pane, "i");
    r.owner
        .wait(&pane, "owner composes", crate::app_mode::writing);
    for n in 1..=4 {
        let text = format!("owner {n}");
        r.owner.typed(&pane, &text);
        r.owner.submit(&pane, &text, n);
    }
    let keys = r.keys();
    no_delivery(&r, &r.relay("four owners block relay"), &keys, 4);
    let last = r.asks().pop().expect("owner fourth").reference.expect("id");
    ae::console::submit::close_owned(&r.owner.tool.dir, &r.owner.home, Some(&last), || Ok(()))
        .expect("close owner fourth");
    r.owner.typed(&pane, "owner private draft");
    let id = r.ask("fourth overall, first relay");
    assert!(
        r.owner
            .draft_row(&r.owner.screen(&pane))
            .contains("owner private draft")
    );
    r.owner.key(&pane, "C-u");
    let keys = r.keys();
    let out = r.relay("blocked by shared total four");
    no_delivery(&r, &out, &keys, 5);
    assert!(words(&out).contains("cap-full"));
    ae::console::submit::close_owned(&r.owner.tool.dir, &r.owner.home, Some(&id), || Ok(()))
        .expect("owner closes local ask");
    let id2 = r.ask("close freed a shared slot");
    assert_eq!(
        ae::console::lane::open_asks(&events(&r.owner.tool.dir), &r.owner.home).len(),
        4
    );
    r.reply(&id2, "reply frees same shared count");
    let id3 = r.ask("reply freed a shared slot");
    ae::console::submit::close_owned(&r.owner.tool.dir, &r.owner.home, Some(&id3), || Ok(()))
        .expect("owner closes relay");
    r.owner.typed(&pane, "fourth owner ask");
    r.owner.submit(&pane, "fourth owner ask", 8);
    let keys = r.keys();
    no_delivery(&r, &r.relay("four owner asks block relay"), &keys, 8);
}
