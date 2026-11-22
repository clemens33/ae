//! S1a HTTP parser acceptance. Wire limits come from plan.md and the agreed framing.

use ae::web::http::{self, Method, Parsed};

const HOST_ROW: &str = "Host: localhost:7480\r\n";
const LINE_CAP: usize = 8 * 1024;
const HEADER_CAP: usize = 16 * 1024;
const BODY_CAP: usize = 64 * 1024;

fn wire(line: &str, fields: &str) -> String {
    format!("{line}\r\n{fields}\r\n")
}

fn get_line(bytes: usize) -> String {
    let path = "p".repeat(bytes - "GET / HTTP/1.1".len());
    format!("GET /{path} HTTP/1.1")
}

fn header_block(bytes: usize) -> String {
    let value = "v".repeat(bytes - HOST_ROW.len() - "X-Pad: \r\n".len());
    format!("{HOST_ROW}X-Pad: {value}\r\n")
}

fn complete(wire: &str) -> (String, usize, usize, usize) {
    match http::parse_head(wire.as_bytes()) {
        Ok(Parsed::Done {
            head,
            head_len,
            body_len,
        }) => (head.target, head_len, body_len, head.headers.len()),
        _ => panic!("a complete valid request head of {} bytes", wire.len()),
    }
}

#[test]
fn request_line_and_header_byte_caps_are_independent_and_inclusive() {
    let header_fields = header_block(HEADER_CAP);
    let request = wire(&get_line(LINE_CAP), &header_fields);
    assert_eq!(
        complete(&request),
        (
            format!("/{}", "p".repeat(LINE_CAP - 14)),
            request.len(),
            0,
            2
        )
    );
    let oversized_line = wire(&get_line(LINE_CAP + 1), &header_fields);
    assert!(http::parse_head(oversized_line.as_bytes()).is_err());
    let oversized_header = wire(&get_line(LINE_CAP), &header_block(HEADER_CAP + 1));
    assert!(http::parse_head(oversized_header.as_bytes()).is_err());
}

#[test]
fn header_field_count_is_bounded_even_when_the_bytes_fit() {
    let mut fields = String::from(HOST_ROW);
    for index in 0..63 {
        use std::fmt::Write as _;
        let _ = write!(fields, "X-Probe-{index}: v\r\n");
    }
    let allowed = wire("GET / HTTP/1.1", &fields);
    assert_eq!(complete(&allowed).3, 64);
    fields.push_str("X-Another: v\r\n");
    let refused = wire("GET / HTTP/1.1", &fields);
    assert!(refused.len() < HEADER_CAP);
    assert!(http::parse_head(refused.as_bytes()).is_err());
}

#[test]
fn a_complete_head_delimits_the_body_and_a_partial_head_does_not() {
    let fields = format!("{HOST_ROW}Content-Length: {BODY_CAP}\r\n");
    let request = wire("POST /api/session HTTP/1.1", &fields);
    let partial = &request.as_bytes()[..request.len() - 1];
    assert!(matches!(http::parse_head(partial), Ok(Parsed::Partial)));

    let whole = format!("{request}the body starts here");
    let Ok(Parsed::Done {
        head,
        head_len,
        body_len,
    }) = http::parse_head(whole.as_bytes())
    else {
        panic!("parse the capped POST without waiting for its full body");
    };
    assert!(matches!(head.method, Method::Post));
    assert_eq!(head.target, "/api/session");
    assert!(
        head.headers
            .iter()
            .any(|(name, value)| { name == "host" && value == "localhost:7480" })
    );
    assert_eq!((head_len, body_len), (request.len(), BODY_CAP));
    assert_eq!(&whole[head_len..], "the body starts here");

    let too_long = format!("{HOST_ROW}Content-Length: {}\r\n", BODY_CAP + 1);
    let request = wire("POST /api/session HTTP/1.1", &too_long);
    assert!(http::parse_head(request.as_bytes()).is_err());
}

#[test]
fn ambiguous_or_unframed_heads_do_not_become_requests() {
    for fields in [
        "Content-Length: 0\r\ncontent-length: 0\r\n",
        "Content-Length: +0\r\n",
        "Content-Length: 0, 0\r\n",
        "Content-Length: 18446744073709551616\r\n",
        "Content-Length: 0\r\nTransfer-Encoding: chunked\r\n",
        "Content-Length: 0\r\n Transfer-Encoding: chunked\r\n",
        "Content-Length: 0\r\nUn Safe: 1\r\n",
        "Content-Length: 0\r\nX-Value: \0\r\n",
    ] {
        let request = wire("POST /api/session HTTP/1.1", &format!("{HOST_ROW}{fields}"));
        assert!(http::parse_head(request.as_bytes()).is_err(), "{fields:?}");
    }

    let control = wire("GET / HTTP/1.1", HOST_ROW);
    assert!(matches!(
        http::parse_head(control.as_bytes()),
        Ok(Parsed::Done { .. })
    ));
    for request in [
        control.replace("\r\n", "\n"),
        wire("GET / HTTP/1.0", HOST_ROW),
        wire("HEAD / HTTP/1.1", HOST_ROW),
        wire("POST /api/session HTTP/1.1", HOST_ROW),
    ] {
        assert!(http::parse_head(request.as_bytes()).is_err(), "{request:?}");
    }
}

#[test]
fn json_body_accepts_the_literal_object_and_rejects_damaged_bytes() {
    let valid = br#"{"code":"ab01","note":"a\n\t\u00e9"}"#;
    let body = http::parse_json_body(valid).unwrap_or_else(|_| panic!("valid JSON object"));
    assert_eq!(body.get_str("code"), Some("ab01"));
    assert_eq!(body.get_str("note"), Some("a\n\té"));
    for bytes in [
        b"".as_slice(),
        br#"{"code":"x""#,
        br#"{"code":"x",}"#,
        br#"{"code":"x"}{}"#,
        b"{\"code\":\"\xff\"}",
    ] {
        assert!(http::parse_json_body(bytes).is_err(), "{bytes:?}");
    }
}
