#![no_main]

use ae::json::Value;
use ae::web::http::{self, Method, Parsed};
use libfuzzer_sys::fuzz_target;

// `ae web`'s readers of untrusted request bytes. The first byte picks one,
// the rest is what it reads:
//
// * 0: a request head, as a socket hands it over, possibly cut short. A whole
//   head ends at its first empty row, whose bytes are all `head_len` counts; a
//   body is announced only by a POST and never beyond the cap; field names are
//   lowercase tokens, and re-reading the head alone gives the same answer. A
//   refusal is one of the statuses the subset answers.
// * 1: a JSON request body; only an object is accepted.
fuzz_target!(|data: &[u8]| {
    let Some((kind, rest)) = data.split_first() else {
        return;
    };
    match kind % 2 {
        0 => head(rest),
        _ => body(rest),
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
            assert!(head_len <= data.len(), "the head lies within the bytes read");
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
