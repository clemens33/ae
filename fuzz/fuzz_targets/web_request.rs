#![no_main]

use ae::json::Value;
use ae::web::claim::{self as web_claim, Word};
use ae::web::http::{self, Method, Parsed};
use libfuzzer_sys::fuzz_target;

// `ae web`'s readers of untrusted bytes. The first byte picks one,
// the rest is what it reads:
//
// * 0: a request head, as a socket hands it over, possibly cut short. A whole
//   head ends at its first empty row, whose bytes are all `head_len` counts; a
//   body is announced only by a POST and never beyond the cap; field names are
//   lowercase tokens, and re-reading the head alone gives the same answer. A
//   refusal is one of the statuses the subset answers.
// * 1: a JSON request body; only an object is accepted.
// * 2: one web claim file as the disk holds it, possibly hand-edited. An
//   accepted record is at most 4 KiB, its two hashes are 64 lowercase hex,
//   its id is a request id, a refusal carries its words, and its encoding
//   reads back the same.
fuzz_target!(|data: &[u8]| {
    let Some((kind, rest)) = data.split_first() else {
        return;
    };
    match kind % 3 {
        0 => head(rest),
        1 => body(rest),
        _ => claim(rest),
    }
});

fn head(data: &[u8]) {
    match std::hint::black_box(http::parse_head(data)) {
        Ok(Parsed::Partial) => {}
        Ok(Parsed::Done {
            head,
            head_len,
            body_len,
        }) => {
            assert!(
                head_len <= data.len(),
                "the head lies within the bytes read"
            );
            let whole = data.get(..head_len).unwrap_or_default();
            assert!(whole.ends_with(b"\r\n\r\n"), "a head ends at its empty row");
            assert!(body_len <= http::BODY_MAX, "a body is capped");
            assert!(
                body_len == 0 || head.method == Method::Post,
                "only a POST announces a body"
            );
            for (name, _) in &head.headers {
                assert!(
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_graphic() && !byte.is_ascii_uppercase()),
                    "a field name is a lowercase token: {name:?}"
                );
            }
            assert_eq!(
                http::parse_head(whole),
                Ok(Parsed::Done {
                    head,
                    head_len,
                    body_len,
                }),
                "the head alone reads the same"
            );
        }
        Err(status) => assert!(
            matches!(status, 400 | 405 | 411 | 413 | 414 | 431 | 505),
            "an unexpected refusal status {status}"
        ),
    }
}

fn body(data: &[u8]) {
    if let Ok(value) = std::hint::black_box(http::parse_json_body(data)) {
        assert!(matches!(value, Value::Obj(_)), "only an object is accepted");
    }
}

fn claim(data: &[u8]) {
    let Ok(read) = std::hint::black_box(web_claim::decode(data)) else {
        return;
    };
    assert!(data.len() <= web_claim::CAP, "a record is capped");
    for hash in [&read.payload, &read.route] {
        assert!(
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "a hash is 64 lowercase hex: {hash:?}"
        );
    }
    assert!(
        ae::tracked::is_request_id(&read.id),
        "the id is a request id: {:?}",
        read.id
    );
    if let Some(outcome) = &read.outcome {
        assert!(
            outcome.word != Word::Refused || !outcome.words.is_empty(),
            "a refusal carries its words"
        );
    }
    let text = web_claim::encode(&read);
    assert_eq!(
        web_claim::decode(text.as_bytes()),
        Ok(read),
        "the encoding reads back the same"
    );
}
