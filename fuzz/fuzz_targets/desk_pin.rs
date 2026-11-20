#![no_main]

use ae::json::{self, Value};
use ae::telegram::{Board, DeskFailure, board_outcome, encode_shown, parse_pin, parse_shown};
use libfuzzer_sys::fuzz_target;

// The Telegram board's three readers of untrusted bytes. The first byte picks
// one, the rest is what it reads:
//
// * 0: the checkpoint `desk-pin`, hand-editable persisted state the bridge
//   reads on every pass, where a file that does not parse holds the board. The
//   grammar has one spelling per checkpoint, so one that parses must encode
//   back to the very bytes read, and its message id is a positive i64.
// * 1: Telegram's answer to a board call: two bytes of HTTP status, a byte
//   that picks the call, the rest the body. `Gone` and `NotModified` need an
//   edit, status 400, an envelope with `ok:false` and `error_code` 400 and
//   their words; a well-formed such answer is always read as that class; only
//   a 2xx answer with `ok:true` is accepted; only a 429 asks to wait, and
//   never past an hour; a status outside 200..=499 is transient.
// * 2: the sidecar `desk-pin.shown`, bound to one chat and message: text it
//   reads for that binding must encode back to the very text read.
fuzz_target!(|data: &[u8]| {
    let Some((kind, rest)) = data.split_first() else {
        return;
    };
    match kind % 3 {
        0 => pin(rest),
        1 => answer(rest),
        _ => shown(rest),
    }
});

fn pin(data: &[u8]) {
    if let Ok(pin) = std::hint::black_box(parse_pin(data)) {
        assert_eq!(
            pin.encode().as_bytes(),
            data,
            "a parsed checkpoint encodes back to itself"
        );
        if let Some(placed) = pin.placed {
            assert!(placed.message > 0, "a message id is positive");
        }
    }
}

fn answer(data: &[u8]) {
    let (head, body) = data.split_at(data.len().min(3));
    let status = match head {
        [high, low, ..] => u16::from_be_bytes([*high, *low]),
        _ => 0,
    };
    let call = match head.get(2).copied().unwrap_or(0) % 3 {
        0 => Board::Send,
        1 => Board::Edit,
        _ => Board::Pin,
    };
    let parsed = std::str::from_utf8(body)
        .ok()
        .and_then(|text| json::parse(text).ok());
    let plain = parsed.as_ref().is_some_and(|answer| {
        answer.get("ok") == Some(&Value::Bool(false))
            && answer.get("error_code") == Some(&Value::Num(400))
    });
    let said = |words: &str| {
        parsed
            .as_ref()
            .and_then(|answer| answer.get_str("description"))
            .is_some_and(|text| text.to_ascii_lowercase().contains(words))
    };
    let edit_400 = status == 400 && call == Board::Edit && plain;
    let not_modified = edit_400 && said("message is not modified");
    let gone = edit_400 && !not_modified && said("message to edit not found");
    let outcome = std::hint::black_box(board_outcome(status, body, call));
    match &outcome {
        Ok(accepted) => {
            assert!((200..300).contains(&status), "only a 2xx is accepted");
            assert_eq!(accepted.get("ok"), Some(&Value::Bool(true)));
        }
        Err(DeskFailure::Gone) => assert!(gone, "Gone needs its status, envelope and words"),
        Err(DeskFailure::NotModified) => assert!(not_modified, "NotModified needs its words"),
        Err(DeskFailure::RetryAfter(seconds)) => {
            assert!(status == 429 && *seconds <= 3600, "only a 429 asks to wait");
        }
        Err(DeskFailure::Transient | DeskFailure::Refused) => {}
    }
    assert_eq!(gone, outcome == Err(DeskFailure::Gone));
    assert_eq!(not_modified, outcome == Err(DeskFailure::NotModified));
    if !(200..=499).contains(&status) {
        assert_eq!(outcome, Err(DeskFailure::Transient));
    }
}

fn shown(data: &[u8]) {
    let Some((pick, rest)) = data.split_first() else {
        return;
    };
    let chat = ["-1001234567890", "@board_chat"][usize::from(*pick & 1)];
    let (id, rest) = rest.split_at(rest.len().min(8));
    let message = i64::from_be_bytes(std::array::from_fn(|at| id.get(at).copied().unwrap_or(0)))
        .saturating_abs();
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    if let Some(body) = std::hint::black_box(parse_shown(text, chat, message)) {
        assert_eq!(
            encode_shown(chat, message, body),
            text,
            "a sidecar read for its binding encodes back to itself"
        );
    }
}
