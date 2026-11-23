//! Acceptance of the durable Telegram board. Only Telegram HTTP is simulated.

use std::fs;

use super::super::desk::{Machine, PIN_FILE, SHOWN_FILE};
use super::{CHAT, FAKE_TOKEN, Fake, Reply, Seen, Temp};
use crate::desk::{Desk, parse, parse_input};
use crate::json::{self, Value};
use crate::time::Timestamp;

const FIRST: &str = concat!(
    "ae-desk 1\nupdated 2026-11-20T08:40:12Z\n",
    "D7\tChoose <east> & ship\thttps://example.test/p?a=1&b=2\t/tmp/board/a & b.md\n",
);
const PIN_OK: &str = r#"{"ok":true,"result":true}"#;
const EDIT_GONE: &str =
    r#"{"ok":false,"error_code":400,"description":"Bad Request: message to edit not found"}"#;
const NOT_MODIFIED: &str = r#"{"ok":false,"error_code":400,"description":"Bad Request: message is not modified: same text"}"#;

fn by_id(id: i64) -> Reply {
    Reply::json(
        200,
        &format!(r#"{{"ok":true,"result":{{"message_id":{id}}}}}"#),
    )
}

fn pin_ok() -> Reply {
    Reply::json(200, PIN_OK)
}

fn ours(text: &str, epoch: i64) -> Desk {
    let input = format!("DECIDE\nD7 | {text}\n");
    Desk {
        updated: Timestamp::from_epoch(epoch),
        items: parse_input(input.as_bytes()).expect("fixture items"),
    }
}

fn wire(seen: &Seen, endpoint: &str, id: Option<i64>) -> Value {
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.path, format!("/bot{FAKE_TOKEN}/{endpoint}"));
    let body = json::parse(&seen.body).expect("request body JSON");
    assert_eq!(body.get("chat_id"), Some(&Value::Num(-1_001_234_567_890)));
    assert_eq!(
        body.get("message_id"),
        id.as_ref().map(|&id| Value::Num(id)).as_ref()
    );
    if endpoint == "pinChatMessage" {
        assert_eq!(body.get("disable_notification"), Some(&Value::Bool(true)));
        assert!(body.get("text").is_none());
    } else {
        assert_eq!(body.get_str("parse_mode"), Some("HTML"));
        assert_eq!(
            body.get("link_preview_options")
                .and_then(|item| item.get("is_disabled")),
            Some(&Value::Bool(true))
        );
        let text = body.get_str("text").expect("rendered message");
        assert!(!text.is_empty() && text.chars().count() <= 4096);
    }
    body
}

#[test]
fn a_near_full_manual_pin_keeps_its_entries_and_stays_within_the_budget() {
    use super::super::desk::FleetRows;
    use std::fmt::Write as _;

    let mut input = "DECIDE\n".to_owned();
    for (id, ampersands) in [(1, 160), (2, 160), (3, 160), (4, 160), (5, 122)] {
        writeln!(input, "D{id} | {}", "&".repeat(ampersands)).expect("fixture item");
    }
    let board = Desk {
        updated: Timestamp::from_epoch(1_795_165_612),
        items: parse_input(input.as_bytes()).expect("valid full manual board"),
    };
    let fleet = FleetRows {
        as_of: 1_795_166_000,
        rows: vec!["birch  running workers: 1 · open asks to you: 0".to_owned()],
    };
    let dir = Temp::new("desk-wide-manual");
    let mut machine = Machine::new(dir.path().to_owned());
    let fake = Fake::start(vec![by_id(42), pin_ok()]);
    let mut err = Vec::new();
    let _ = machine.pass_with_fleet(
        &fake.api(),
        CHAT,
        Some(&board),
        Some(&fleet),
        fleet.as_of,
        &mut err,
    );
    let received = fake.requests();
    assert_eq!(received.len(), 2, "one send and one pin");
    let sent = wire(&received[0], "sendMessage", None);
    let text = sent.get_str("text").expect("manual board reached Telegram");
    assert!(
        text.contains(&format!("D5 {}", "&amp;".repeat(122))),
        "the last manual item survives the live append"
    );
    let units = text.encode_utf16().count();
    assert!(units <= 3900, "the pin contains {units} UTF-16 units");
}

#[test]
fn an_available_live_sample_cannot_create_a_pin_without_a_manual_record() {
    use super::super::desk::FleetRows;

    let dir = Temp::new("desk-live-no-record");
    let mut machine = Machine::new(dir.path().to_owned());
    let fake = Fake::start(vec![by_id(42), pin_ok()]);
    let api = fake.api();
    let mut err = Vec::new();
    let start = 1_795_164_012;
    let live = FleetRows {
        as_of: start,
        rows: vec!["dispatch  lead  blocked  since 08:30  needs an answer".to_owned()],
    };

    let _ = machine.pass_with_fleet(&api, CHAT, None, Some(&live), start, &mut err);
    assert!(fake.requests().is_empty(), "no manual record means no pin");
    assert!(!dir.path().join(PIN_FILE).exists());

    let board = ours("A manual decision exists", start);
    let _ = machine.pass_with_fleet(&api, CHAT, Some(&board), Some(&live), start, &mut err);
    let requests = fake.requests();
    assert_eq!(
        requests.len(),
        2,
        "the same live data can extend a real pin"
    );
    let sent = wire(&requests[0], "sendMessage", None);
    let text = sent.get_str("text").expect("pin text");
    assert!(text.contains("A manual decision exists") && text.contains("needs an answer"));
    wire(&requests[1], "pinChatMessage", Some(42));
}

#[test]
fn one_pin_survives_restart_edits_and_refusals_until_an_edit_proves_it_gone() {
    let dir = Temp::new("desk-life");
    let mut machine = Machine::new(dir.path().to_owned());
    let fake = Fake::start(vec![by_id(42), pin_ok()]);
    let api = fake.api();
    let mut err = Vec::new();
    let first = parse(FIRST.as_bytes()).expect("first record");

    let _ = machine.pass(&api, CHAT, None, 1000, &mut err);
    assert!(fake.requests().is_empty(), "no board produces no calls");
    let _ = machine.pass(&api, CHAT, Some(&first), 1001, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&first), 1002, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 2, "one send and one pin: {requests:?}");
    let sent = wire(&requests[0], "sendMessage", None);
    let text = sent.get_str("text").expect("send text");
    for fragment in [
        "DECIDE",
        "D7",
        "08:40",
        "Choose &lt;east&gt; &amp; ship",
        r#"<a href="https://example.test/p?a=1&amp;b=2">"#,
        "<code>/tmp/board/a &amp; b.md</code>",
    ] {
        assert!(text.contains(fragment), "missing {fragment:?} in {text:?}");
    }
    wire(&requests[1], "pinChatMessage", Some(42));
    let mut machine = Machine::new(dir.path().to_owned());
    let _ = machine.pass(&api, CHAT, Some(&first), 1003, &mut err);
    assert_eq!(fake.requests().len(), 2, "unchanged after restart");

    let second = ours("Ship the east region", 1004);
    fake.script(vec![by_id(42)]);
    let _ = machine.pass(&api, CHAT, Some(&second), 1004, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 3);
    let edited = wire(&requests[2], "editMessageText", Some(42));
    let text = edited.get_str("text").expect("edit text");
    assert!(text.contains("Ship the east region"));
    assert!(!text.contains("Choose &lt;east&gt;"));

    let third = ours("Wait for the west region", 1005);
    for (index, (status, response)) in [
        (
            400,
            r#"{"ok":false,"error_code":400,"description":"Bad Request: chat not found"}"#,
        ),
        (
            403,
            r#"{"ok":false,"error_code":403,"description":"Forbidden"}"#,
        ),
        (
            400,
            r#"{"ok":false,"error_code":403,"description":"message to edit not found"}"#,
        ),
        (500, EDIT_GONE),
        (400, "broken response"),
        (
            429,
            r#"{"ok":false,"error_code":429,"parameters":{"retry_after":120}}"#,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        fake.script(vec![Reply::json(status, response)]);
        let now = 2000 + i64::try_from(index).expect("test case") * 1000;
        let _ = machine.pass(&api, CHAT, Some(&third), now, &mut err);
        let requests = fake.requests();
        assert_eq!(requests.len(), 4 + index, "failure {status}: {requests:?}");
        wire(&requests[3 + index], "editMessageText", Some(42));
    }

    fake.script(vec![Reply::json(400, NOT_MODIFIED)]);
    let _ = machine.pass(&api, CHAT, Some(&third), 9000, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 10);
    wire(&requests[9], "editMessageText", Some(42));
    let mut machine = Machine::new(dir.path().to_owned());
    let _ = machine.pass(&api, CHAT, Some(&third), 9001, &mut err);
    assert_eq!(fake.requests().len(), 10, "NotModified was checkpointed");

    let fourth = ours("Schedule the central region", 9002);
    fake.script(vec![Reply::json(400, EDIT_GONE)]);
    let _ = machine.pass(&api, CHAT, Some(&fourth), 9002, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 11);
    wire(&requests[10], "editMessageText", Some(42));
    fake.script(vec![by_id(84), pin_ok()]);
    let _ = machine.pass(&api, CHAT, Some(&fourth), 9003, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&fourth), 9004, &mut err);
    let requests = fake.requests();
    assert_eq!(
        requests.len(),
        13,
        "one replacement and its pin: {requests:?}"
    );
    wire(&requests[11], "sendMessage", None);
    wire(&requests[12], "pinChatMessage", Some(84));
    assert!(!String::from_utf8_lossy(&err).contains(FAKE_TOKEN));
}

#[test]
fn a_pending_pin_retries_its_id_after_restart_and_does_not_block_edits() {
    let dir = Temp::new("desk-pending");
    let fake = Fake::start(vec![
        by_id(42),
        Reply::json(
            429,
            r#"{"ok":false,"error_code":429,"parameters":{"retry_after":120}}"#,
        ),
    ]);
    let api = fake.api();
    let mut err = Vec::new();
    let mut machine = Machine::new(dir.path().to_owned());
    let first = parse(FIRST.as_bytes()).expect("first record");
    let _ = machine.pass(&api, CHAT, Some(&first), 1000, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&first), 1001, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    wire(&requests[0], "sendMessage", None);
    wire(&requests[1], "pinChatMessage", Some(42));

    let change = ours("This edit can proceed during pin backoff", 1002);
    fake.script(vec![by_id(42)]);
    let _ = machine.pass(&api, CHAT, Some(&change), 1002, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 3);
    wire(&requests[2], "editMessageText", Some(42));

    let mut machine = Machine::new(dir.path().to_owned());
    fake.script(vec![pin_ok()]);
    let _ = machine.pass(&api, CHAT, Some(&change), 1300, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&change), 1301, &mut err);
    let requests = fake.requests();
    assert_eq!(
        requests.len(),
        4,
        "no new message across restart: {requests:?}"
    );
    wire(&requests[3], "pinChatMessage", Some(42));
}

#[test]
fn unreadable_pin_identity_holds_and_a_wrong_shown_binding_forces_an_edit() {
    let dir = Temp::new("desk-checkpoint");
    let fake = Fake::start(vec![by_id(42), pin_ok()]);
    let api = fake.api();
    let pin = dir.path().join(PIN_FILE);
    let shown = dir.path().join(SHOWN_FILE);
    let first = parse(FIRST.as_bytes()).expect("first record");
    let mut machine = Machine::new(dir.path().to_owned());
    let mut err = Vec::new();

    fs::write(&pin, "ae-desk-pin 1\nchat broken\nmessage no\n").expect("damaged pin");
    let held = machine.pass(&api, CHAT, Some(&first), 1000, &mut err);
    assert!(held.held.is_some() && fake.requests().is_empty());
    assert!(String::from_utf8_lossy(&err).contains("holding"), "{err:?}");
    fs::remove_file(&pin).expect("operator removes the damaged pin");
    let _ = machine.pass(&api, CHAT, Some(&first), 1001, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&first), 1002, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    let sent = wire(&requests[0], "sendMessage", None);
    let text = sent.get_str("text").expect("sent board text");
    fs::write(
        &shown,
        format!("ae-desk-shown 1\nchat {CHAT}\nmessage 77\n{text}"),
    )
    .expect("stale sidecar for a different message");
    fake.script(vec![Reply::json(400, NOT_MODIFIED)]);
    let _ = machine.pass(&api, CHAT, Some(&first), 1003, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&first), 1004, &mut err);
    let requests = fake.requests();
    assert_eq!(
        requests.len(),
        3,
        "stale text from another id cannot suppress an edit"
    );
    wire(&requests[2], "editMessageText", Some(42));
}

#[test]
fn wide_unicode_render_compares_byte_exactly_across_restart() {
    let dir = Temp::new("desk-unicode");
    let fake = Fake::start(vec![by_id(42), pin_ok()]);
    let api = fake.api();
    let mut input = String::from("DECIDE\n");
    let text = "🦦".repeat(160);
    for id in 1..=6 {
        use std::fmt::Write as _;
        writeln!(input, "D{id} | {text}").expect("append to String");
    }
    input.push_str("COMING\nC1 | ");
    input.push_str(&"<".repeat(80));
    input.push('\n');
    let board = Desk {
        updated: Timestamp::from_epoch(1000),
        items: parse_input(input.as_bytes()).expect("valid Unicode board"),
    };
    let mut err = Vec::new();
    let mut machine = Machine::new(dir.path().to_owned());
    let _ = machine.pass(&api, CHAT, Some(&board), 1000, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&board), 1001, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    let body = wire(&requests[0], "sendMessage", None);
    let actual = body.get_str("text").expect("wide Unicode message");
    assert_eq!(actual.matches('🦦').count(), 960);
    assert!(
        actual.len() > 3900,
        "fixture exercises a wide UTF-8 sidecar"
    );
    let mut machine = Machine::new(dir.path().to_owned());
    let _ = machine.pass(&api, CHAT, Some(&board), 1002, &mut err);
    assert_eq!(
        fake.requests().len(),
        2,
        "equal Unicode board is already shown"
    );
}

#[test]
fn telegram_budget_omits_whole_lower_priority_items_and_keeps_valid_html() {
    use super::super::desk::render;
    use std::fmt::Write as _;

    let mut input =
        String::from("DECIDE\nD7 | Decide first | https://example.test/decision\nLINKS\n");
    for id in 1..=12 {
        writeln!(
            input,
            "L{id} | {} | https://example.test/next?row={id}&tab=notes",
            "&".repeat(160)
        )
        .expect("fill the accepted board");
    }
    let board = Desk {
        updated: Timestamp::from_epoch(1000),
        items: parse_input(input.as_bytes()).expect("valid board with expanded HTML"),
    };
    let text = render(&board);
    assert!(text.chars().count() <= 4096);
    assert!(text.contains("Decide first"));
    assert!(text.contains(r#"href="https://example.test/decision""#));
    assert!(text.contains("more") && text.contains("/board"));
    assert!(
        !text.contains("row=12"),
        "lowest-priority tail does not fit"
    );
    assert_eq!(
        text.matches("<a href=").count(),
        text.matches("</a>").count()
    );
    assert_eq!(text.matches("<b>").count(), text.matches("</b>").count());
    assert_eq!(text.matches('<').count(), text.matches('>').count());
}

#[test]
fn an_edit_waits_for_transient_backoff_and_the_server_retry_after() {
    let dir = Temp::new("desk-edit-retry");
    let fake = Fake::start(vec![by_id(42), pin_ok()]);
    let api = fake.api();
    let mut machine = Machine::new(dir.path().to_owned());
    let mut err = Vec::new();
    let first = ours("First published board", 1000);
    let next = ours("Edited after backoff", 1100);
    let _ = machine.pass(&api, CHAT, Some(&first), 1000, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&first), 1001, &mut err);
    assert_eq!(fake.requests().len(), 2);

    fake.script(vec![Reply::json(500, r#"{"ok":false,"error_code":500}"#)]);
    for now in [1100, 1110, 1120] {
        let _ = machine.pass(&api, CHAT, Some(&next), now, &mut err);
    }
    let requests = fake.requests();
    assert_eq!(requests.len(), 5, "three real attempts before the probe");
    for request in &requests[2..] {
        wire(request, "editMessageText", Some(42));
    }
    let _ = machine.pass(&api, CHAT, Some(&next), 1122, &mut err);
    assert_eq!(fake.requests().len(), 5, "the third failure delays an edit");

    fake.script(vec![Reply::json(
        429,
        r#"{"ok":false,"error_code":429,"parameters":{"retry_after":120}}"#,
    )]);
    let _ = machine.pass(&api, CHAT, Some(&next), 1130, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 6);
    wire(&requests[5], "editMessageText", Some(42));
    for now in [1132, 1249] {
        let _ = machine.pass(&api, CHAT, Some(&next), now, &mut err);
        assert_eq!(
            fake.requests().len(),
            6,
            "retry_after still applies at {now}"
        );
    }

    fake.script(vec![by_id(42)]);
    let _ = machine.pass(&api, CHAT, Some(&next), 1251, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 7);
    wire(&requests[6], "editMessageText", Some(42));
}

#[test]
fn a_failed_send_checkpoint_causes_no_second_send_in_the_same_process() {
    let dir = Temp::new("desk-checkpoint-io");
    let fake = Fake::start(vec![by_id(42)]);
    let api = fake.api();
    let mut machine = Machine::new(dir.path().to_owned());
    let mut err = Vec::new();
    let first = ours("Keep the message identity in memory", 1000);
    super::with_unwritable_dir(dir.path(), || {
        for now in [1000, 2000, 3000, 4000, 5000] {
            let _ = machine.pass(&api, CHAT, Some(&first), now, &mut err);
        }
        let requests = fake.requests();
        assert_eq!(
            requests.len(),
            1,
            "Telegram accepted one send and storage still cannot checkpoint it: {requests:?}"
        );
        wire(&requests[0], "sendMessage", None);
        assert!(!dir.path().join(PIN_FILE).exists());
    });

    fake.script(vec![pin_ok()]);
    let _ = machine.pass(&api, CHAT, Some(&first), 6000, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&first), 6001, &mut err);
    let requests = fake.requests();
    assert_eq!(
        requests.len(),
        2,
        "repair permits the saved identity to pin"
    );
    wire(&requests[1], "pinChatMessage", Some(42));
}

#[test]
fn pin_not_found_and_forbidden_keep_the_same_identity() {
    let dir = Temp::new("desk-pin-refusal");
    let fake = Fake::start(vec![
        by_id(42),
        Reply::json(
            400,
            r#"{"ok":false,"error_code":400,"description":"Bad Request: message to pin not found"}"#,
        ),
    ]);
    let api = fake.api();
    let mut machine = Machine::new(dir.path().to_owned());
    let mut err = Vec::new();
    let first = ours("Waiting for pin permissions", 1000);
    let _ = machine.pass(&api, CHAT, Some(&first), 1000, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&first), 1001, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    wire(&requests[0], "sendMessage", None);
    wire(&requests[1], "pinChatMessage", Some(42));

    fake.script(vec![Reply::json(
        403,
        r#"{"ok":false,"error_code":403,"description":"Forbidden: cannot pin messages"}"#,
    )]);
    let _ = machine.pass(&api, CHAT, Some(&first), 2000, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 3);
    wire(&requests[2], "pinChatMessage", Some(42));

    let _ = machine.pass(&api, CHAT, Some(&first), 2000, &mut err);
    assert_eq!(fake.requests().len(), 3, "a refusal enters pin backoff");

    let next = ours("Edited while the pin was refused", 3000);
    fake.script(vec![by_id(42), pin_ok()]);
    let _ = machine.pass(&api, CHAT, Some(&next), 3000, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&next), 3001, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 5);
    wire(&requests[3], "editMessageText", Some(42));
    wire(&requests[4], "pinChatMessage", Some(42));
}

#[test]
fn a_configured_chat_change_creates_and_pins_a_message_in_the_new_chat() {
    let dir = Temp::new("desk-chat-switch");
    let pin = dir.path().join(PIN_FILE);
    fs::write(
        &pin,
        "ae-desk-pin 1\nchat -1001234567891\nmessage 17\npinned 1\n",
    )
    .expect("valid checkpoint for the previous chat");
    let fake = Fake::start(vec![by_id(42), pin_ok()]);
    let api = fake.api();
    let mut machine = Machine::new(dir.path().to_owned());
    let mut err = Vec::new();
    let board = ours("Board for the configured chat", 1000);
    for now in [1000, 1001, 1002] {
        let _ = machine.pass(&api, CHAT, Some(&board), now, &mut err);
    }
    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    wire(&requests[0], "sendMessage", None);
    wire(&requests[1], "pinChatMessage", Some(42));
    assert_eq!(
        fs::read_to_string(pin).expect("new checkpoint"),
        format!("ae-desk-pin 1\nchat {CHAT}\nmessage 42\npinned 1\n")
    );
}

#[test]
fn a_persistent_edit_refusal_backs_off_and_retries_the_same_id() {
    let dir = Temp::new("desk-edit-refused");
    let fake = Fake::start(vec![by_id(42), pin_ok()]);
    let api = fake.api();
    let mut machine = Machine::new(dir.path().to_owned());
    let mut err = Vec::new();
    let first = ours("Last accepted board", 1000);
    let next = ours("Board waiting for edit permission", 1100);
    let _ = machine.pass(&api, CHAT, Some(&first), 1000, &mut err);
    let _ = machine.pass(&api, CHAT, Some(&first), 1001, &mut err);
    assert_eq!(fake.requests().len(), 2);

    fake.script(vec![Reply::json(
        403,
        r#"{"ok":false,"error_code":403,"description":"Forbidden"}"#,
    )]);
    for now in [1100, 1110, 1120] {
        let _ = machine.pass(&api, CHAT, Some(&next), now, &mut err);
    }
    let requests = fake.requests();
    assert_eq!(requests.len(), 5);
    for request in &requests[2..] {
        wire(request, "editMessageText", Some(42));
    }
    let _ = machine.pass(&api, CHAT, Some(&next), 1122, &mut err);
    assert_eq!(fake.requests().len(), 5, "a persistent refusal backs off");

    fake.script(vec![by_id(42)]);
    let _ = machine.pass(&api, CHAT, Some(&next), 1130, &mut err);
    let requests = fake.requests();
    assert_eq!(requests.len(), 6);
    wire(&requests[5], "editMessageText", Some(42));
}

#[test]
fn live_pin_cadence_tracks_the_last_confirmed_display_even_after_restart() {
    use super::super::desk::FleetRows;

    let temp = Temp::new("desk-live-clock");
    let fake = Fake::start(vec![by_id(91), pin_ok()]);
    let api = fake.api();
    let mut err = Vec::new();
    let mut machine = Machine::new(temp.path().to_owned());
    let start = 1_795_164_012;
    let desk = ours("Manual decision remains", start - 1000);
    let rows = |at, reason: &str| FleetRows {
        as_of: at,
        rows: vec![format!("dispatch  lead  blocked  since 08:30  {reason}")],
    };
    let first = rows(start, "first reason");
    let _ = machine.pass_with_fleet(&api, CHAT, Some(&desk), Some(&first), start, &mut err);
    let later = rows(start + 6, "first reason");
    let _ = machine.pass_with_fleet(&api, CHAT, Some(&desk), Some(&later), start + 6, &mut err);
    let initial = fake.requests();
    assert_eq!(initial.len(), 2);
    let original = wire(&initial[0], "sendMessage", None);
    let original = original.get_str("text").expect("first pin text");
    assert!(
        original.contains("LIVE") && original.contains("first reason"),
        "{original}"
    );
    wire(&initial[1], "pinChatMessage", Some(91));

    let mut machine = Machine::new(temp.path().to_owned());
    let unchanged = rows(start + 252, "first reason");
    let _ = machine.pass_with_fleet(
        &api,
        CHAT,
        Some(&desk),
        Some(&unchanged),
        start + 299,
        &mut err,
    );
    assert_eq!(
        fake.requests().len(),
        2,
        "a newer sample alone does not spend an edit early, even after restart"
    );

    fake.script(vec![by_id(91)]);
    let _ = machine.pass_with_fleet(
        &api,
        CHAT,
        Some(&desk),
        Some(&unchanged),
        start + 300,
        &mut err,
    );
    let requests = fake.requests();
    assert_eq!(
        requests.len(),
        3,
        "a quiet pin still refreshes in five minutes"
    );
    let edited = wire(&requests[2], "editMessageText", Some(91));
    let edited = edited.get_str("text").expect("periodic edit text");
    assert_ne!(edited, original, "the as-of visible to the owner moves");
    assert!(edited.contains("first reason") && edited.contains("Manual decision remains"));

    let relevant = rows(start + 301, "relevant changed reason");
    fake.script(vec![by_id(91)]);
    let _ = machine.pass_with_fleet(
        &api,
        CHAT,
        Some(&desk),
        Some(&relevant),
        start + 301,
        &mut err,
    );
    let requests = fake.requests();
    assert_eq!(
        requests.len(),
        4,
        "a new reason refreshes before the next periodic edit"
    );
    let edited = wire(&requests[3], "editMessageText", Some(91));
    let edited = edited.get_str("text").expect("relevant edit text");
    assert!(edited.contains("relevant changed reason"), "{edited}");
}
