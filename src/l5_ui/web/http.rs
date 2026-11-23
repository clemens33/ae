//! The HTTP/1.1 subset `ae web` speaks. PURE: bytes in, values out; the
//! socket lives in `server.rs`.
//!
//! A request is one head and, for a POST, one `Content-Length` body. The head
//! is read under fixed limits counted in wire bytes, and anything outside the
//! subset is refused by status: no chunked body, no keep-alive, no upgrade, no
//! ranges. Every response closes its connection and carries the pinned
//! security headers byte for byte.

use std::sync::Arc;

use crate::json::{self, Value};

/// The most bytes of the request line, its CRLF not counted.
pub const LINE_MAX: usize = 8192;
/// The most bytes of all header field rows, each counted with its CRLF and the
/// empty delimiter row not counted.
pub const FIELDS_MAX: usize = 16384;
/// The most header field rows.
pub const FIELD_ROWS_MAX: usize = 64;
/// The most bytes of a request body.
pub const BODY_MAX: usize = 65536;

/// The Content-Security-Policy every response carries.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
connect-src 'self'; img-src 'self'; manifest-src 'self'; base-uri 'none'; \
form-action 'none'; frame-ancestors 'none'";

/// The headers a request may carry once at most.
const SINGLE: [&str; 5] = [
    "content-length",
    "host",
    "origin",
    "authorization",
    "if-none-match",
];

/// The two methods the subset answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
}

/// A request head that parsed whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The method.
    pub method: Method,
    /// The request target as sent: an origin-form path and query.
    pub target: String,
    /// Each header field in order: its name lowercased, its value trimmed.
    pub headers: Vec<(String, String)>,
}

impl Head {
    /// The value of header `name` (lowercase), if the request sent it.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }

    /// The path of the target, its query cut off.
    #[must_use]
    pub fn path(&self) -> &str {
        self.target
            .split_once('?')
            .map_or(self.target.as_str(), |(path, _)| path)
    }
}

/// How far the bytes read so far make a request head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// More bytes are needed; nothing seen so far is refused.
    Partial,
    /// The head is whole: it ends at `head_len` and `body_len` bytes follow.
    Done {
        /// The head.
        head: Head,
        /// Its length in bytes, the empty delimiter row included.
        head_len: usize,
        /// The body length its `Content-Length` names; 0 without one.
        body_len: usize,
    },
}

/// Parse the request head at the start of `buf`, the bytes read so far.
///
/// # Errors
///
/// The status that refuses the request: 400 a malformed head, 405 another
/// method, 411 a body without `Content-Length` or any `Transfer-Encoding`, 413
/// a body over [`BODY_MAX`], 414 a request line over [`LINE_MAX`], 431 header
/// fields over [`FIELDS_MAX`] or [`FIELD_ROWS_MAX`], 505 a version other than
/// `HTTP/1.1`.
pub fn parse_head(buf: &[u8]) -> Result<Parsed, u16> {
    bare_lf(buf)?;
    let Some(line_end) = find_crlf(buf, 0) else {
        // The line may still end in a CR already read.
        return if buf.len() > LINE_MAX + 1 {
            Err(414)
        } else {
            Ok(Parsed::Partial)
        };
    };
    if line_end > LINE_MAX {
        return Err(414);
    }
    let (method, target) = request_line(buf.get(..line_end).ok_or(400_u16)?)?;
    let mut at = line_end + 2;
    let mut fields = 0_usize;
    let mut headers = Vec::new();
    loop {
        let Some(end) = find_crlf(buf, at) else {
            return if fields + (buf.len() - at) > FIELDS_MAX + 1 {
                Err(431)
            } else {
                Ok(Parsed::Partial)
            };
        };
        if end == at {
            at += 2;
            break;
        }
        fields += end - at + 2;
        if fields > FIELDS_MAX || headers.len() == FIELD_ROWS_MAX {
            return Err(431);
        }
        headers.push(field(buf.get(at..end).ok_or(400_u16)?)?);
        at = end + 2;
    }
    let head = Head {
        method,
        target,
        headers,
    };
    let body_len = body_len(&head)?;
    Ok(Parsed::Done {
        head,
        head_len: at,
        body_len,
    })
}

/// Refuse a LF that no CR precedes in the head: the bytes through the first
/// empty row, or all of `buf` while that row has not arrived. A body read in
/// the same segment is never scanned, so how TCP splits a request cannot
/// change its answer.
fn bare_lf(buf: &[u8]) -> Result<(), u16> {
    let head = buf
        .windows(4)
        .position(|row| row == b"\r\n\r\n")
        .and_then(|at| buf.get(..at + 4))
        .unwrap_or(buf);
    let bare = head
        .iter()
        .enumerate()
        .any(|(at, byte)| *byte == b'\n' && (at == 0 || head.get(at - 1) != Some(&b'\r')));
    if bare { Err(400) } else { Ok(()) }
}

/// The index of the next CRLF in `buf` at or after `from`.
fn find_crlf(buf: &[u8], from: usize) -> Option<usize> {
    buf.get(from..)?
        .windows(2)
        .position(|pair| pair == b"\r\n")
        .map(|at| at + from)
}

/// The method and target of a request line.
fn request_line(line: &[u8]) -> Result<(Method, String), u16> {
    let text = std::str::from_utf8(line).map_err(|_| 400_u16)?;
    let mut parts = text.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(400);
    };
    if method.is_empty() || !method.bytes().all(is_tchar) {
        return Err(400);
    }
    let visible = |byte: u8| (0x21..=0x7e).contains(&byte);
    if !target.starts_with('/') || !target.bytes().all(visible) {
        return Err(400);
    }
    if version != "HTTP/1.1" {
        return Err(if version.starts_with("HTTP/") {
            505
        } else {
            400
        });
    }
    let method = match method {
        "GET" => Method::Get,
        "POST" => Method::Post,
        _ => return Err(405),
    };
    Ok((method, target.to_owned()))
}

/// One header field row: its lowercased name and its trimmed value.
fn field(row: &[u8]) -> Result<(String, String), u16> {
    if matches!(row.first(), Some(b' ' | b'\t')) {
        return Err(400);
    }
    let colon = row.iter().position(|byte| *byte == b':').ok_or(400_u16)?;
    let (name, value) = (row.get(..colon), row.get(colon + 1..));
    let (Some(name), Some(value)) = (name, value) else {
        return Err(400);
    };
    if name.is_empty() || !name.iter().copied().all(is_tchar) {
        return Err(400);
    }
    let plain = |byte: &u8| *byte == b'\t' || (0x20..=0x7e).contains(byte);
    if !value.iter().all(plain) {
        return Err(400);
    }
    let value = std::str::from_utf8(value).map_err(|_| 400_u16)?;
    let name = std::str::from_utf8(name).map_err(|_| 400_u16)?;
    Ok((
        name.to_ascii_lowercase(),
        value.trim_matches([' ', '\t']).to_owned(),
    ))
}

/// Whether `byte` is a `tchar` of RFC 9110 §5.6.2.
fn is_tchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// The body length the head names, after the single-header and framing rules.
fn body_len(head: &Head) -> Result<usize, u16> {
    for name in SINGLE {
        if head
            .headers
            .iter()
            .filter(|(field, _)| field == name)
            .count()
            > 1
        {
            return Err(400);
        }
    }
    if head.header("transfer-encoding").is_some() {
        return Err(411);
    }
    let length = match head.header("content-length") {
        None if head.method == Method::Post => return Err(411),
        None => 0,
        Some(text) => {
            if text.is_empty() || text.len() > 9 || !text.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(400);
            }
            text.parse::<usize>().map_err(|_| 400_u16)?
        }
    };
    if length > BODY_MAX {
        return Err(413);
    }
    if head.method == Method::Get && length > 0 {
        return Err(400);
    }
    Ok(length)
}

/// A request body as the JSON object every API POST sends.
///
/// # Errors
///
/// 400 when the body is not UTF-8, not JSON or not an object.
pub fn parse_json_body(body: &[u8]) -> Result<Value, u16> {
    let text = std::str::from_utf8(body).map_err(|_| 400_u16)?;
    match json::parse(text) {
        Ok(value @ Value::Obj(_)) => Ok(value),
        _ => Err(400),
    }
}

/// One response: its status, its media type and body, and its entity tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The media type; ignored for 304.
    pub content_type: &'static str,
    /// The body, shared with the snapshot it may come from; ignored for 304.
    pub body: Arc<[u8]>,
    /// The `ETag` value, quotes included.
    pub etag: Option<String>,
}

/// The media type of every JSON answer.
pub const JSON: &str = "application/json";

impl Response {
    /// A JSON answer of `value` with status `status`.
    #[must_use]
    pub fn json(status: u16, value: &Value) -> Self {
        Self {
            status,
            content_type: JSON,
            body: value.render().into_bytes().into(),
            etag: None,
        }
    }

    /// The refusal `{"error": words}` with status `status`.
    #[must_use]
    pub fn refusal(status: u16, words: &str) -> Self {
        Self::json(status, &Value::obj([("error", Value::str(words))]))
    }
}

/// The reason phrase of `status`.
#[must_use]
pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        413 => "Content Too Large",
        414 => "URI Too Long",
        421 => "Misdirected Request",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        _ => "Error",
    }
}

/// The wire bytes of `response` before its body: status line,
/// `Content-Type`, `Content-Length`, the pinned headers, the `ETag`,
/// `Connection: close` and the empty row. A 304 carries no media type or
/// length.
#[must_use]
pub fn head(response: &Response) -> Vec<u8> {
    use std::fmt::Write as _;

    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason(response.status)
    );
    if response.status != 304 {
        let _ = write!(
            head,
            "Content-Type: {}\r\nContent-Length: {}\r\n",
            response.content_type,
            response.body.len()
        );
    }
    head.push_str("Cache-Control: no-store\r\n");
    let _ = write!(head, "Content-Security-Policy: {CSP}\r\n");
    head.push_str("X-Content-Type-Options: nosniff\r\n");
    head.push_str("Referrer-Policy: no-referrer\r\n");
    head.push_str("Cross-Origin-Opener-Policy: same-origin\r\n");
    head.push_str("Cross-Origin-Resource-Policy: same-origin\r\n");
    if let Some(etag) = &response.etag {
        let _ = write!(head, "ETag: {etag}\r\n");
    }
    head.push_str("Connection: close\r\n\r\n");
    head.into_bytes()
}

/// The body bytes `response` sends: none for a 304.
#[must_use]
pub fn body(response: &Response) -> &[u8] {
    if response.status == 304 {
        &[]
    } else {
        &response.body
    }
}

#[cfg(test)]
mod tests {
    use super::{Parsed, parse_head};

    #[test]
    fn a_body_read_with_its_head_is_never_scanned_for_bare_line_feeds() {
        let head = b"POST /api/session HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: 4\r\n\r\n"
            .to_vec();
        let mut whole = head.clone();
        whole.extend_from_slice(b"{\n}\n");
        for buf in [&head, &whole] {
            assert!(matches!(
                parse_head(buf),
                Ok(Parsed::Done { body_len: 4, .. })
            ));
        }
        assert_eq!(
            parse_head(b"GET / HTTP/1.1\r\nHost: a\nX: b\r\n\r\n").err(),
            Some(400)
        );
        assert_eq!(parse_head(b"GET / HTTP/1.1\n").err(), Some(400));
    }

    #[test]
    fn every_status_the_listener_sends_carries_its_registered_reason_phrase() {
        // RFC 9110 section 15 and RFC 6585 section 5 (431), not this module.
        for (status, phrase) in [
            (200, "OK"),
            (202, "Accepted"),
            (304, "Not Modified"),
            (400, "Bad Request"),
            (401, "Unauthorized"),
            (403, "Forbidden"),
            (404, "Not Found"),
            (405, "Method Not Allowed"),
            (408, "Request Timeout"),
            (409, "Conflict"),
            (410, "Gone"),
            (411, "Length Required"),
            (413, "Content Too Large"),
            (414, "URI Too Long"),
            (421, "Misdirected Request"),
            (431, "Request Header Fields Too Large"),
            (503, "Service Unavailable"),
            (505, "HTTP Version Not Supported"),
        ] {
            assert_eq!(super::reason(status), phrase, "{status}");
        }
    }

    /// The contract of `parse_head`: 414 only for a request line over
    /// `LINE_MAX`, and a head still arriving is `Partial`, refusing nothing.
    #[test]
    fn a_request_line_of_exactly_line_max_is_never_refused_while_it_arrives() {
        let line = format!("GET /{} HTTP/1.1", "a".repeat(super::LINE_MAX - 14));
        assert_eq!(line.len(), super::LINE_MAX);
        let head = format!("{line}\r\nHost: 127.0.0.1:1\r\n\r\n");
        for end in line.len()..head.len() {
            let part = head
                .as_bytes()
                .get(..end)
                .unwrap_or_else(|| panic!("{end} is inside the head"));
            assert_eq!(parse_head(part), Ok(Parsed::Partial), "{end} bytes");
        }
        assert!(matches!(
            parse_head(head.as_bytes()),
            Ok(Parsed::Done { body_len: 0, .. })
        ));
        let over = format!("GET /{} HTTP/1.1\r\n", "a".repeat(super::LINE_MAX - 13));
        assert_eq!(parse_head(over.as_bytes()), Err(414), "one byte over");
    }
}
