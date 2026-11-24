//! S3a decisions through the authenticated state, topic and send endpoints.

use std::sync::Arc;

use ae::json::{self, Value};
use ae::web::state::{self, Read};
use ae::web::topics::{self, Sticky};

use super::{
    CARRY_WAIT, Cut, DAY, Draft, EPOCH, EXPIRED, Flow, Lookup, Pause, Reply, Rig, TestClock, Web,
    Word, await_evidence, confirmed, is_hex, keep_bearer_through, lookup, openings, post, role_rig,
    send_wire, text,
};

const ITEM: &str = "D67";
const QUESTION: &str = "Choose 🧭 #release #notes today";
const OLD: &str = "DECIDE\nD67 | Choose 🧭 #release #notes today\n";
const NEW: &str = "DECIDE\nD67 | A new question under the same id\n";
const CHANGED: &str = "the decision changed, reload";

fn board(rig: &Rig, input: &str) {
    let items = ae::desk::parse_input(input.as_bytes()).expect("fixture board grammar");
    ae::deskstore::set(
        &rig.dir,
        items,
        ae::time::Timestamp::from_epoch(EPOCH),
        "tui",
    )
    .expect("publish the fixture board");
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    let Some(Value::Arr(items)) = value.get(key) else {
        panic!("missing array {key}: {}", value.render());
    };
    items
}

fn get(web: &Web, path: &str) -> Value {
    let reply = web.get(path);
    assert_eq!(reply.status, 200, "{path}");
    reply.value
}

fn desk(value: &Value) -> &Value {
    value.get("desk").expect("desk in state")
}

fn decide(value: &Value) -> &[Value] {
    array(desk(value), "decide")
}

fn bind(web: &Web) -> String {
    let state = get(web, "/api/state");
    let row = decide(&state)
        .iter()
        .find(|row| row.get_str("item") == Some(ITEM))
        .expect("D67 in state");
    let bind = text(row, "bind");
    assert!(is_hex(bind, 64), "{bind}");
    bind.to_owned()
}

fn one_wait(row: &Value, key: &str, item: &str, question: &str) {
    let waits = array(row, key);
    assert_eq!(waits.len(), 1, "{}", row.render());
    let wait = &waits[0];
    assert_eq!(wait.get_str("kind"), Some("decide"));
    assert_eq!(wait.get_str("item"), Some(item));
    assert_eq!(wait.get_str("text"), Some(question));
    assert_eq!(wait.get("id"), Some(&Value::Null));
    assert!(wait.get("bind").is_none());
}

#[test]
fn s3a_state_and_cards_keep_decision_identity_and_board_order() {
    let rig = role_rig("ws3state");
    let memo = "2026-11-01T00:00:00Z\ttui\ttopic-release\ttitle: Release\n\
                2026-11-01T00:00:00Z\ttui\ttopic-notes\ttitle: Notes\n";
    ae::store::open(&rig.dir)
        .append_memo(memo.as_bytes())
        .expect("fixture topics");
    board(
        &rig,
        &format!("{OLD}D2 | No tag\nRUNNING\nR1 | Running #release\n"),
    );
    let clock = TestClock::new();
    let web = Web::live(&rig, &clock, 0);
    let value = get(&web, "/api/state");
    let decisions = decide(&value);
    assert_eq!(
        decisions
            .iter()
            .map(|row| text(row, "item"))
            .collect::<Vec<_>>(),
        ["D2", ITEM],
    );
    assert_eq!(text(&decisions[1], "text"), QUESTION);
    assert_eq!(
        array(&decisions[1], "tags"),
        [Value::str("release"), Value::str("notes")]
    );
    assert!(array(&decisions[0], "tags").is_empty());
    assert!(decisions.iter().all(|row| is_hex(text(row, "bind"), 64)));
    let first_bind = text(&decisions[1], "bind");
    assert_ne!(text(&decisions[0], "bind"), first_bind);
    let compose = desk(&value).get("compose").expect("compose limits");
    for (name, limit) in [("slug_max", 48), ("title_max", 80), ("answer_max", 120)] {
        assert_eq!(compose.get(name), Some(&Value::Num(limit)));
    }
    one_wait(desk(&value), "untagged", "D2", "No tag");
    for slug in ["release", "notes"] {
        let row = array(desk(&value), "groups")
            .iter()
            .flat_map(|group| array(group, "rows"))
            .find(|row| row.get_str("slug") == Some(slug))
            .expect("tagged topic row");
        one_wait(row, "waits", ITEM, QUESTION);
        one_wait(
            &get(&web, &format!("/api/topic/{slug}")),
            "waits",
            ITEM,
            QUESTION,
        );
    }

    // Independently calculated HMAC-SHA256 for the documented message and key.
    let read = Read {
        desk: topics::collect(&rig.scratch, &mut Sticky::default()),
        ..Read::default()
    };
    let rendered = state::render(Arc::new(read), "fixed key", EPOCH, &[0x42; 32]);
    let rendered_value = json::parse(std::str::from_utf8(&rendered.body).expect("UTF-8 state"))
        .expect("rendered state JSON");
    assert_eq!(
        text(&decide(&rendered_value)[1], "bind"),
        "6180c0c1291160cf5c81308554db602f03611a09efec5bedb4ed902d6f95f83d",
    );

    clock.advance(1);
    assert_eq!(bind(&web), first_bind);
    let other = Web::live(&rig, &clock, 0);
    assert_ne!(bind(&other), first_bind);

    board(&rig, NEW);
    clock.advance(1);
    assert_ne!(bind(&web), first_bind);

    use std::fmt::Write as _;
    let mut full = String::from("DECIDE\n");
    for number in (1..=16).rev() {
        writeln!(full, "D{number} | Decision {number}").expect("format fixture");
    }
    board(&rig, &full);
    clock.advance(1);
    let full = get(&web, "/api/state");
    assert_eq!(decide(&full).len(), 16);
    for (index, row) in decide(&full).iter().enumerate() {
        assert_eq!(text(row, "item"), format!("D{}", index + 1));
    }
}

fn payload(draft: &Draft, item: &str, binding: &str) -> Value {
    Value::obj([
        ("text", Value::str(draft.body.as_str())),
        ("route", Value::str(draft.route.as_str())),
        ("nonce", Value::str(draft.nonce.as_str())),
        (
            "decide",
            Value::obj([("item", Value::str(item)), ("bind", Value::str(binding))]),
        ),
    ])
}

fn submit(web: &Web, body: &Value) -> Reply {
    post(web.addr, "/api/send", body, &web.token)
}

fn raw(web: &Web, body: &str) -> Reply {
    let host = web.addr;
    let token = &web.token;
    let len = body.len();
    send_wire(
        host,
        &format!(
            "POST /api/send HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {len}\r\n\r\n{body}"
        ),
    )
}

fn settled(web: &Web, body: &Value) -> Reply {
    await_evidence(CARRY_WAIT, || {
        let reply = submit(web, body);
        (reply.status != 202).then_some(reply)
    })
    .expect("decision draft settles")
}

#[test]
fn s3a_decide_shape_rejects_bad_metadata_and_accepts_the_scalar_edge() {
    let rig = role_rig("ws3shape");
    board(&rig, OLD);
    let time = TestClock::new();
    let web = Web::live(&rig, &time, 0);
    let route = web.route(&rig);
    let binding = bind(&web);
    let mut draft = web.draft(&route, "D67 1");
    for bad in [
        "D68 1".to_owned(),
        "D671".to_owned(),
        "D67".to_owned(),
        "D67 ".to_owned(),
        "D67   ".to_owned(),
        " D67 1".to_owned(),
        "D67  1".to_owned(),
        "D67 1 ".to_owned(),
        "D67 .\u{00a0}".to_owned(),
        "D67 \u{3000}line".to_owned(),
        "D67 \u{2003}line".to_owned(),
        "D67 line\u{202f}".to_owned(),
        "D67 one\ntwo".to_owned(),
        "D67 one\rtwo".to_owned(),
        "D67 one\ttwo".to_owned(),
        "D67 one\u{007f}two".to_owned(),
        "D67 one\u{0085}two".to_owned(),
        "D67 one\u{2028}two".to_owned(),
        "D67 one\u{2029}two".to_owned(),
        format!("D67 {}", "🧭".repeat(121)),
    ] {
        draft.body = bad;
        let reply = submit(&web, &payload(&draft, ITEM, &binding));
        assert_eq!(
            reply.status,
            400,
            "accepted {:#?}: {}",
            draft.body,
            reply.value.render()
        );
        assert!(matches!(lookup(&rig, &draft), Lookup::Absent));
    }
    for invalid in [
        "R1", "C3", "W2", "L1", "D0", "D01", "D100", "d67", "D 67", "",
    ] {
        draft.body = format!("{invalid} 1");
        assert_eq!(
            submit(&web, &payload(&draft, invalid, &binding)).status,
            400,
            "{invalid}"
        );
    }
    draft.body = "D67 1".to_owned();
    for invalid in [
        "0".repeat(63),
        "0".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
    ] {
        assert_eq!(
            submit(&web, &payload(&draft, ITEM, &invalid)).status,
            400,
            "{invalid}"
        );
    }

    let valid = payload(&draft, ITEM, &binding).render();
    let fields = valid.strip_suffix('}').expect("outer JSON object");
    let proof = format!(r#"{{"item":"{ITEM}","bind":"{binding}"}}"#);
    let plain = draft.json().render();
    let plain_fields = plain.strip_suffix('}').expect("plain JSON object");
    for invalid in [
        format!(r#"{fields},"extra":true}}"#),
        format!(r#"{fields},"decide":{proof}}}"#),
        format!(r#"{plain_fields},"decide":null}}"#),
        format!(r#"{plain_fields},"decide":[]}}"#),
        format!(r#"{plain_fields},"decide":{{"item":"D67","bind":"{binding}","extra":true}}}}"#),
        format!(r#"{plain_fields},"decide":{{"item":"D67","item":"D67","bind":"{binding}"}}}}"#),
        format!(
            r#"{plain_fields},"decide":{{"item":"D67","bind":"{binding}","bind":"{binding}"}}}}"#
        ),
        format!(r#"{plain_fields},"decide":{{"item":"D67"}}}}"#),
    ] {
        assert_eq!(raw(&web, &invalid).status, 400, "{invalid}");
        assert!(matches!(lookup(&rig, &draft), Lookup::Absent));
    }
    assert_eq!(rig.enter_count(), 0);
    assert!(openings(&rig).is_empty());

    draft.body = format!("D67 \u{feff}{}\u{00a0}\u{feff}", "🧭".repeat(117));
    let body = payload(&draft, ITEM, &binding);
    submit(&web, &body).expect(202, "state", "queued");
    let id = confirmed(&settled(&web, &body), &rig);
    assert_eq!(rig.enter_count(), 1);
    assert_eq!(openings(&rig), [id.as_str()]);
    assert!(
        rig.submitted()
            .contains(&format!("REQUEST {id} from human:chat: {}\n", draft.body))
    );
}

#[test]
fn s3a_queue_refuses_missing_replaced_and_unreadable_boards_without_spending() {
    let rig = role_rig("ws3stale");
    board(&rig, OLD);
    let time = TestClock::new();
    let web = Web::live(&rig, &time, 0);
    let draft = web.draft(&web.route(&rig), "D67 1");
    let body = payload(&draft, ITEM, &bind(&web));
    let record = rig.dir.join(ae::desk::RECORD_FILE);

    for replacement in [Some(NEW), Some("DECIDE\nD68 | An unrelated item\n"), None] {
        if let Some(replacement) = replacement {
            board(&rig, replacement);
        } else {
            std::fs::remove_file(&record).expect("remove fixture board");
        }
        submit(&web, &body).expect(409, "error", CHANGED);
        assert!(matches!(lookup(&rig, &draft), Lookup::Absent));
    }
    std::fs::write(&record, "bad board record\n").expect("malformed fixture");
    submit(&web, &body).expect(409, "error", CHANGED);
    assert!(matches!(lookup(&rig, &draft), Lookup::Absent));
    assert_eq!(rig.enter_count(), 0);
    assert!(openings(&rig).is_empty());

    std::fs::remove_file(&record).expect("clear malformed fixture");
    board(&rig, OLD);
    submit(&web, &body).expect(202, "state", "queued");
    let receipt = settled(&web, &body);
    let id = confirmed(&receipt, &rig);
    assert!(
        rig.submitted()
            .contains(&format!("REQUEST {id} from human:chat: D67 1\n"))
    );

    board(&rig, NEW);
    let retry = submit(&web, &body);
    assert_eq!(retry.status, 200);
    assert_eq!(retry.value, receipt.value);
    assert_eq!(rig.enter_count(), 1);
    assert_eq!(openings(&rig), [id]);
}

#[test]
fn s3a_changed_board_does_not_displace_pending_drafts_and_settles_carry_as_changed() {
    let rig = role_rig("ws3carry");
    board(&rig, OLD);
    let time = TestClock::new();
    let (hooks, pause) = Pause::at(Cut::Claimed);
    let web = Web::with_hook(&rig, &time, hooks);
    let pause = pause;
    let route = web.route(&rig);
    let binding = bind(&web);
    let first = web.draft(&route, "D67 1");
    let body = payload(&first, ITEM, &binding);
    submit(&web, &body).expect(202, "state", "queued");
    let claimed_id = pause.reached();

    let next = web.draft(&route, "D67 2");
    let pending = payload(&next, ITEM, &binding);
    submit(&web, &pending).expect(202, "state", "queued");
    assert!(matches!(lookup(&rig, &next), Lookup::Absent));
    board(&rig, NEW);
    submit(&web, &body).expect(202, "state", "queued");
    submit(&web, &pending).expect(202, "state", "queued");
    assert!(matches!(lookup(&rig, &next), Lookup::Absent));
    pause.finish(Flow::Go);

    for (draft, payload) in [(&first, &body), (&next, &pending)] {
        settled(&web, payload).expect(409, "error", CHANGED);
        let Lookup::Known(claim) = lookup(&rig, draft) else {
            panic!("a carry-time refusal must leave a claim");
        };
        if draft.nonce == first.nonce {
            assert_eq!(claim.id, claimed_id);
        }
        let outcome = claim.outcome.expect("claim settled");
        assert!(matches!(outcome.word, Word::Changed));
        assert_eq!(outcome.words, CHANGED);
    }
    board(&rig, OLD);
    submit(&web, &body).expect(409, "error", CHANGED);
    assert_eq!(rig.enter_count(), 0);
    assert!(openings(&rig).is_empty());
}

#[test]
fn s3a_nonce_verification_precedes_board_checks_and_binding_is_listener_local() {
    let rig = role_rig("ws3key");
    board(&rig, OLD);
    let time = TestClock::new();
    let first = Web::live(&rig, &time, 0);
    let second = Web::live(&rig, &time, 0);
    let first_bind = bind(&first);
    let second_bind = bind(&second);
    let route = second.route(&rig);
    let local = second.draft(&route, "D67 1");

    submit(&second, &payload(&local, ITEM, &first_bind)).expect(409, "error", CHANGED);
    assert!(matches!(lookup(&rig, &local), Lookup::Absent));
    let foreign = first.draft(&route, "D67 1");
    board(&rig, NEW);
    submit(&second, &payload(&foreign, ITEM, &second_bind)).expect(410, "error", EXPIRED);
    keep_bearer_through(&second, &time, DAY + 1);
    submit(&second, &payload(&local, ITEM, &second_bind)).expect(410, "error", EXPIRED);
    assert_eq!(rig.enter_count(), 0);
    assert!(openings(&rig).is_empty());
}

#[test]
fn s3a_docs_name_the_owner_lines_undo_charter_and_cap() {
    let docs = include_str!("../../../docs/web.md");
    for expected in [
        "D67 1",
        "#<slug> new <title>",
        "Undo",
        "5 s",
        "charter",
        "D20",
    ] {
        assert!(docs.contains(expected), "web docs missing {expected:?}");
    }
    assert!(docs.contains("five open asks") || docs.contains("5 open asks"));
}
