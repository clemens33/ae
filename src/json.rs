//! Minimal JSON, hand-written, both directions.
//!
//! Hand-written rather than a dependency for three reasons, in order of weight:
//!
//! 1. **SC-510d names the escape set** ae writes (`\"` `\\` `\n` `\t` `\r`).
//!    A contract about bytes on disk is a contract this crate should own.
//! 2. **SC-506 is satisfied by construction** when rendering is infallible: a
//!    [`Value`] tree renders to a `String` that always closes, so no
//!    per-session failure can truncate the document mid-array.
//! 3. #80's "no dependency exists until a real one does" — and the first real
//!    dependency also costs the `--allow license-not-encountered` relaxation in
//!    `just rust-deny`, which is a deliberate change, not a side effect.
//!
//! The parse half is deliberately tolerant: SC-511b says readers ignore keys
//! they do not understand, and SC-511c says the event schema grows by adding
//! optional keys. A parser that knew only today's keys would break on the first
//! additive change it is contractually required to survive.

use std::fmt;
use std::fmt::Write as _;

/// A JSON value, in the subset ae's own formats use.
///
/// Objects keep their fields as an ordered list rather than a map so rendering
/// is deterministic: [`Value::obj`] preserves insertion order instead of
/// hashing. That is a property of this type, not a schema contract — list-digest
/// member order is an open choice (phase-3 criterion 15 / SC-509).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A string.
    Str(String),
    /// An integer. ae's own numbers are `schema_version`, epochs and ranks —
    /// all integral, so this is the only numeric shape the emitter builds.
    Num(i64),
    /// A numeric literal this crate does not interpret — a float, an exponent,
    /// or an integer too large for [`Value::Num`] — kept verbatim.
    ///
    /// It exists for the READ direction only. SC-511c says the schema grows by
    /// adding optional keys, and SC-511b says a reader ignores the ones it does
    /// not understand: refusing a line because some future key carried `1.5`
    /// would break exactly the compatibility those rows promise.
    Raw(String),
    /// A boolean.
    Bool(bool),
    /// Null.
    Null,
    /// An array.
    Arr(Vec<Value>),
    /// An object, in field order.
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// Build an object from an iterator of `(key, value)` pairs.
    ///
    /// ```
    /// use ae::json::Value;
    /// let v = Value::obj([("schema_version", Value::Num(1))]);
    /// assert_eq!(v.render(), r#"{"schema_version":1}"#);
    /// ```
    #[must_use]
    pub fn obj<K: Into<String>, I: IntoIterator<Item = (K, Value)>>(fields: I) -> Self {
        Self::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// Build a string value.
    #[must_use]
    pub fn str<S: Into<String>>(s: S) -> Self {
        Self::Str(s.into())
    }

    /// The string behind a [`Value::Str`], or `None` for every other shape.
    ///
    /// Readers use this rather than matching: an unknown key whose value is a
    /// number is *ignored*, not an error (SC-511b).
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Look a field up by key. `None` unless `self` is an object holding it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A field's string value, or `None` when absent or not a string.
    #[must_use]
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Self::as_str)
    }

    /// Render into `out`. Infallible — see the SC-506 note on this module.
    pub fn render_into(&self, out: &mut String) {
        match self {
            Self::Str(s) => {
                out.push('"');
                escape_into(s, out);
                out.push('"');
            }
            Self::Num(n) => {
                let _ = write!(out, "{n}");
            }
            Self::Raw(token) => out.push_str(token),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Null => out.push_str("null"),
            Self::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.render_into(out);
                }
                out.push(']');
            }
            Self::Obj(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    escape_into(key, out);
                    out.push_str("\":");
                    value.render_into(out);
                }
                out.push('}');
            }
        }
    }

    /// Render to a fresh `String`.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.render_into(&mut out);
        out
    }

    /// True when both values carry the same members and values, ignoring object
    /// field order. Arrays remain order-sensitive.
    ///
    /// List-digest member *order* is an open choice (phase-3 criterion 15).
    /// Tests that need the member set compare with this rather than `==` or
    /// rendered bytes. Insertion-order preservation is a separate determinism
    /// property of [`Value::obj`].
    #[cfg(test)]
    #[must_use]
    pub(crate) fn same_members(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Obj(left), Self::Obj(right)) => {
                left.len() == right.len()
                    && left.iter().all(|(key, value)| {
                        right.iter().any(|(other_key, other_value)| {
                            other_key == key && value.same_members(other_value)
                        })
                    })
            }
            (Self::Arr(left), Self::Arr(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(left, right)| left.same_members(right))
            }
            (left, right) => left == right,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// Escape `s` into `out` as the body of a JSON string (no surrounding quotes).
///
/// SC-510d pins the set ae *writes*: `\"` `\\` `\n` `\t` `\r`. The remaining C0
/// control bytes are escaped as `\u00XX` because a raw control byte inside a
/// string is not legal JSON, and AGENTS.md's JSON-emitter row calls for
/// handling control bytes at write time — escaping preserves the byte while
/// keeping the document parseable.
pub fn escape_into(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other if (other as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(other));
            }
            other => out.push(other),
        }
    }
}

/// Why a line could not be read as JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset into the input where the parser gave up.
    pub at: usize,
    /// What the parser wanted there.
    pub wanted: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid json at byte {}: expected {}",
            self.at, self.wanted
        )
    }
}

impl std::error::Error for ParseError {}

/// Parse one complete JSON value, allowing leading and trailing whitespace.
///
/// # Errors
///
/// Returns [`ParseError`] when the input is not exactly one complete JSON value.
///
/// ```
/// let v = ae::json::parse(r#"{"action":"done"}"#)?;
/// assert_eq!(v.get_str("action"), Some("done"));
/// # Ok::<(), ae::json::ParseError>(())
/// ```
pub fn parse(input: &str) -> Result<Value, ParseError> {
    let mut parser = Parser {
        src: input.as_bytes(),
        pos: 0,
    };
    parser.skip_ws();
    let value = parser.value(0)?;
    parser.skip_ws();
    if parser.pos == parser.src.len() {
        Ok(value)
    } else {
        Err(parser.err("end of input"))
    }
}

/// How deep a value may nest before the parser refuses it.
///
/// Not a contract row: a recursive-descent parser recurses, and an event log is
/// a file on disk that a defect elsewhere could fill with `[[[[…`. A bounded
/// refusal degrades one line; a stack overflow aborts the process, which is the
/// one thing SC-506 says the document must never do.
const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, wanted: &'static str) -> ParseError {
        ParseError {
            at: self.pos,
            wanted,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, byte: u8, wanted: &'static str) -> Result<(), ParseError> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(wanted))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.err("less deeply nested json"));
        }
        match self.peek() {
            Some(b'"') => self.string().map(Value::Str),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.err("a json value")),
        }
    }

    fn literal(&mut self, word: &'static str, value: Value) -> Result<Value, ParseError> {
        if self.src[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.err("a json literal"))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.eat(b'{', "an object")?;
        let mut fields = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Obj(fields));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.eat(b':', "a colon")?;
            self.skip_ws();
            fields.push((key, self.value(depth + 1)?));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Obj(fields));
                }
                _ => return Err(self.err("a comma or a closing brace")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.eat(b'[', "an array")?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Arr(items));
                }
                _ => return Err(self.err("a comma or a closing bracket")),
            }
        }
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let digits_from = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.pos == digits_from {
            return Err(self.err("a digit"));
        }
        // RFC 8259: int = "0" / (digit1-9 *DIGIT). A leading zero is not a
        // stylistic quirk to tolerate — `010` is octal in several languages and
        // 10 in others, so accepting it means accepting a value whose meaning
        // depends on who reads it next. `0` and `-0` are legal; `01` is not.
        if self.pos - digits_from > 1 && self.src.get(digits_from) == Some(&b'0') {
            return Err(self.err("no leading zero"));
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            let from = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == from {
                return Err(self.err("a digit after the decimal point"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let from = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == from {
                return Err(self.err("a digit in the exponent"));
            }
        }
        let token = self.slice(start, self.pos)?;
        // An integer that fits is a number this crate understands; anything
        // else is carried verbatim rather than refused (SC-511b/c). No separate
        // "is it integral" flag is needed: `i64::from_str` accepts only an
        // optional sign and digits, so every float and exponent lands in the
        // fallback by itself.
        token
            .parse::<i64>()
            .map_or_else(|_| Ok(Value::Raw(token.to_owned())), |n| Ok(Value::Num(n)))
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.eat(b'"', "a string")?;
        let mut out = String::new();
        loop {
            let byte = self.peek().ok_or_else(|| self.err("a closing quote"))?;
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    self.escape(&mut out)?;
                }
                0x00..=0x1f => return Err(self.err("an escaped control character")),
                _ => {
                    let end = self.pos + utf8_len(byte);
                    out.push_str(self.slice(self.pos, end)?);
                    self.pos = end;
                }
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<(), ParseError> {
        let byte = self.peek().ok_or_else(|| self.err("an escape character"))?;
        self.pos += 1;
        match byte {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => out.push(self.unicode_escape()?),
            _ => return Err(self.err("a known escape character")),
        }
        Ok(())
    }

    /// The `\uXXXX` form, including the surrogate pair that carries an
    /// astral character. A lone surrogate is refused rather than replaced:
    /// it is not a `char`, and silently substituting one would put bytes in
    /// the model that were never in the file.
    fn unicode_escape(&mut self) -> Result<char, ParseError> {
        let first = self.hex4()?;
        let code = if (0xD800..=0xDBFF).contains(&first) {
            self.eat(b'\\', "a low surrogate escape")?;
            self.eat(b'u', "a low surrogate escape")?;
            let second = self.hex4()?;
            if !(0xDC00..=0xDFFF).contains(&second) {
                return Err(self.err("a low surrogate"));
            }
            0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00)
        } else {
            first
        };
        char::from_u32(code).ok_or_else(|| self.err("a valid unicode escape"))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let end = self.pos + 4;
        let digits = self.slice(self.pos, end)?;
        let value = u32::from_str_radix(digits, 16).map_err(|_| self.err("four hex digits"))?;
        self.pos = end;
        Ok(value)
    }

    fn slice(&self, from: usize, to: usize) -> Result<&str, ParseError> {
        let bytes = self
            .src
            .get(from..to)
            .ok_or_else(|| self.err("more input"))?;
        std::str::from_utf8(bytes).map_err(|_| self.err("valid utf-8"))
    }
}

/// How many bytes the UTF-8 character starting with `lead` occupies.
///
/// The input is a `&str`, so the byte is always a legal lead byte; a
/// continuation byte would only be reached through a bug here, and `1` keeps
/// that bug a parse error instead of a panic.
fn utf8_len(lead: u8) -> usize {
    match lead {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        // ASCII, and — only reachable through a bug here — a continuation byte.
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{escape_into, parse, Value};

    #[test]
    fn sc_510d_escapes_the_documented_set() {
        // SC-510d: the escape set is \" \\ \n \t \r.
        let mut out = String::new();
        escape_into("a\"b\\c\nd\te\rf", &mut out);
        assert_eq!(out, r#"a\"b\\c\nd\te\rf"#);
    }

    #[test]
    fn a_control_byte_outside_the_documented_set_still_leaves_legal_json() {
        // AGENTS.md JSON-emitter row: a control byte must not reach the
        // document raw, or the line stops being parseable JSON.
        let mut out = String::new();
        escape_into("a\u{0}b\u{1f}c", &mut out);
        assert_eq!(out, r"a\u0000b\u001fc");
    }

    #[test]
    fn ordinary_text_survives_escaping_unchanged() {
        let mut out = String::new();
        escape_into("plain text - unicode 😀 / slash", &mut out);
        assert_eq!(out, "plain text - unicode 😀 / slash");
    }

    #[test]
    fn objects_render_in_field_order() {
        // Determinism of this type: `Value::obj` preserves insertion order
        // rather than hashing. A HashMap-backed object would make two renders
        // of the same construction incomparable. List-digest member order is a
        // separate, open choice (phase-3 criterion 15); this test does not
        // document a schema order.
        let v = Value::obj([
            ("schema_version", Value::Num(1)),
            ("generated_at", Value::str("2026-05-29T14:00:00Z")),
            ("sessions", Value::Arr(vec![])),
        ]);
        assert_eq!(
            v.render(),
            r#"{"schema_version":1,"generated_at":"2026-05-29T14:00:00Z","sessions":[]}"#
        );
    }

    #[test]
    fn object_member_equality_ignores_field_order() {
        let left = Value::obj([("a", Value::Num(1)), ("b", Value::Num(2))]);
        let right = Value::obj([("b", Value::Num(2)), ("a", Value::Num(1))]);
        assert!(
            left.same_members(&right),
            "the same members in either order are the same document"
        );
        assert_ne!(
            left, right,
            "PartialEq on this type stays order-sensitive: that is why tests that \
             must not pin order go through same_members"
        );
    }

    #[test]
    fn every_scalar_shape_renders() {
        assert_eq!(Value::Bool(true).render(), "true");
        assert_eq!(Value::Bool(false).render(), "false");
        assert_eq!(Value::Null.render(), "null");
        assert_eq!(Value::Num(-17).render(), "-17");
        assert_eq!(Value::str("hi").render(), r#""hi""#);
    }

    #[test]
    fn nested_arrays_and_objects_render() {
        let v = Value::Arr(vec![
            Value::obj([("a", Value::Num(1))]),
            Value::Arr(vec![Value::Null]),
        ]);
        assert_eq!(v.render(), r#"[{"a":1},[null]]"#);
    }

    #[test]
    fn a_documented_event_line_round_trips() {
        // The events.md schema block, in its documented key order.
        let line = concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"done","#,
            r#""target":"codex:coworker","ref":"ae-20260519T072100Z-abc123","#,
            r#""summary":"first 200 chars"}"#
        );
        let parsed = parse(line).expect("the documented event shape must parse");
        assert_eq!(parsed.get_str("ts"), Some("2026-05-19T07:29:45Z"));
        assert_eq!(parsed.get_str("actor"), Some("claude:lead"));
        assert_eq!(parsed.get_str("action"), Some("done"));
        assert_eq!(parsed.get_str("ref"), Some("ae-20260519T072100Z-abc123"));
        assert_eq!(parsed.render(), line);
    }

    #[test]
    fn sc_511b_unknown_keys_of_any_type_are_tolerated() {
        // SC-511b: readers ignore keys they do not understand. SC-511c: the
        // schema grows by ADDING optional keys — one day, keys whose values are
        // not strings.
        let line = r#"{"ts":"t","actor":"a","action":"send","future":{"n":[1,2,null,true]}}"#;
        let parsed = parse(line).expect("an additive key must not break the reader");
        assert_eq!(parsed.get_str("actor"), Some("a"));
        assert_eq!(
            parsed.get_str("future"),
            None,
            "a non-string is not a string"
        );
        assert!(parsed.get("future").is_some(), "but it is still present");
    }

    #[test]
    fn escape_sequences_are_decoded_on_the_way_in() {
        let parsed = parse(r#"{"summary":"a\"b\\c\nd\te\rf\/gA"}"#).expect("escapes parse");
        assert_eq!(parsed.get_str("summary"), Some("a\"b\\c\nd\te\rf/gA"));
    }

    #[test]
    fn surrogate_pairs_decode_to_one_character() {
        // The ESCAPED form. An emoji typed literally here would exercise the
        // utf-8 copy path and never reach the \u decoder at all — which is what
        // this test did before cargo-mutants walked past the whole branch.
        let parsed = parse(r#"{"s":"\ud83d\ude00"}"#).expect("a surrogate pair parses");
        assert_eq!(parsed.get_str("s"), Some("😀"));
    }

    #[test]
    fn a_basic_plane_unicode_escape_decodes() {
        let parsed = parse(r#"{"s":"\u00fc\u0041\u20ac"}"#).expect("unicode escapes parse");
        assert_eq!(parsed.get_str("s"), Some("üA€"));
    }

    #[test]
    fn a_broken_unicode_escape_is_refused_rather_than_guessed() {
        for broken in [
            r#"{"s":"\u00"}"#,         // too few digits
            r#"{"s":"\uzzzz"}"#,       // not hex
            r#"{"s":"\ud83d"}"#,       // a high surrogate with no partner
            r#"{"s":"\ud83dA"}"#,      // a high surrogate followed by anything else
            r#"{"s":"\ud83d\u0041"}"#, // a high surrogate followed by a non-surrogate
            r#"{"s":"\udc00"}"#,       // a lone LOW surrogate
        ] {
            assert!(parse(broken).is_err(), "{broken} must not parse");
        }
    }

    #[test]
    fn the_two_escapes_ae_never_writes_are_still_understood() {
        // SC-510d pins what ae WRITES. A reader that met \b or \f from any
        // other producer should decode it, not reject the line.
        let parsed =
            parse(r#"{"s":"a\bb\fc"}"#).expect("the backspace and form-feed escapes parse");
        assert_eq!(parsed.get_str("s"), Some("a\u{8}b\u{c}c"));
    }

    #[test]
    fn multi_byte_text_inside_a_string_is_copied_whole() {
        // Two-, three- and four-byte sequences, unescaped.
        let parsed = parse("{\"s\":\"ü — 😀\"}").expect("utf-8 parses");
        assert_eq!(parsed.get_str("s"), Some("ü — 😀"));
    }

    #[test]
    fn an_empty_object_parses_and_round_trips() {
        assert_eq!(parse("{}"), Ok(Value::Obj(vec![])));
        assert_eq!(parse("{}").map(|v| v.render()), Ok("{}".to_owned()));
        assert_eq!(parse("{ }"), Ok(Value::Obj(vec![])));
    }

    #[test]
    fn negative_and_exponent_numbers_parse_and_round_trip() {
        let parsed = parse(r#"{"a":-17,"b":2e+3,"c":2e-3,"d":-0.5}"#).expect("numbers parse");
        assert_eq!(parsed.get("a"), Some(&Value::Num(-17)));
        assert_eq!(parsed.get("b"), Some(&Value::Raw("2e+3".to_owned())));
        assert_eq!(parsed.get("c"), Some(&Value::Raw("2e-3".to_owned())));
        assert_eq!(parsed.get("d"), Some(&Value::Raw("-0.5".to_owned())));
        assert_eq!(parsed.render(), r#"{"a":-17,"b":2e+3,"c":2e-3,"d":-0.5}"#);
    }

    #[test]
    fn the_integer_grammar_forbids_a_leading_zero() {
        // Legal: a bare zero, a negative zero, and any zero-led FRACTION.
        for legal in ["0", "-0", "0.5", "-0.5", "0e3", "10", "-10", "100"] {
            assert!(parse(legal).is_ok(), "{legal} is legal json");
        }
        // Illegal: an integer part with a leading zero, anywhere it appears.
        for illegal in ["01", "-01", "00", "-00", "00.1", "012", "0123e4"] {
            let err = parse(illegal).expect_err("leading zeros are not json");
            assert_eq!(err.wanted, "no leading zero", "{illegal}");
        }
    }

    #[test]
    fn a_leading_zero_is_refused_wherever_a_number_may_appear() {
        assert!(parse(r#"{"a":01}"#).is_err());
        assert!(parse("[01]").is_err());
        assert!(parse(r#"{"a":[1,-01]}"#).is_err());
        // And the legal neighbours still parse in the same positions.
        assert!(parse(r#"{"a":0}"#).is_ok());
        assert!(parse("[-0]").is_ok());
    }

    #[test]
    fn a_number_missing_its_digits_is_refused() {
        for broken in ["-", "1.", "1e", "1e+", "{\"a\":-}"] {
            assert!(parse(broken).is_err(), "{broken:?} must not parse");
        }
    }

    #[test]
    fn the_nesting_limit_is_a_boundary_not_a_vibe() {
        // MAX_DEPTH is 64 and the outermost value is depth 0, so 65 nested
        // arrays are the deepest legal input and 66 are one too many.
        let ok = "[".repeat(65) + &"]".repeat(65);
        assert!(parse(&ok).is_ok(), "65 levels must parse");
        let too_deep = "[".repeat(66) + &"]".repeat(66);
        assert_eq!(
            parse(&too_deep).expect_err("66 levels is too deep").wanted,
            "less deeply nested json"
        );
    }

    #[test]
    fn objects_nest_as_deeply_as_arrays_do() {
        // Depth is counted through BOTH containers; a bug that only counted
        // arrays would leave the object path unbounded.
        let deep = "{\"a\":".repeat(200) + "1" + &"}".repeat(200);
        assert_eq!(
            parse(&deep).expect_err("too deep").wanted,
            "less deeply nested json"
        );
    }

    #[test]
    fn whitespace_around_and_inside_a_value_is_allowed() {
        let parsed = parse("  { \"a\" : [ 1 , 2 ] , \"b\" : true }  ").expect("whitespace parses");
        assert_eq!(parsed.get("b"), Some(&Value::Bool(true)));
    }

    #[test]
    fn trailing_content_after_a_complete_value_is_an_error() {
        // A line holding two objects is not one event.
        let err = parse(r#"{"a":1} {"b":2}"#).expect_err("two values is not one value");
        assert_eq!(err.wanted, "end of input");
    }

    #[test]
    fn truncated_input_is_an_error_rather_than_a_partial_value() {
        for broken in [
            r#"{"a":"#,
            r#"{"a":1"#,
            r#"{"a"#,
            "[1,",
            r#""unterminated"#,
            "",
        ] {
            assert!(parse(broken).is_err(), "{broken:?} must not parse");
        }
    }

    #[test]
    fn a_non_object_top_level_value_still_parses() {
        // This is a JSON parser; deciding that an EVENT must be an object is
        // the event reader's job, not the lexer's.
        assert_eq!(parse("[]"), Ok(Value::Arr(vec![])));
        assert_eq!(parse("null"), Ok(Value::Null));
    }

    #[test]
    fn a_number_this_crate_does_not_interpret_is_kept_verbatim() {
        // SC-511c: additive keys are fine — including, one day, a non-integral
        // one. Refusing the line would break the compatibility the row promises.
        let parsed = parse(r#"{"a":1.5,"b":2e3,"c":99999999999999999999}"#)
            .expect("a float-valued additive key must not break the reader");
        assert_eq!(parsed.get("a"), Some(&Value::Raw("1.5".to_owned())));
        assert_eq!(parsed.get("b"), Some(&Value::Raw("2e3".to_owned())));
        assert_eq!(
            parsed.get("c"),
            Some(&Value::Raw("99999999999999999999".to_owned()))
        );
        assert_eq!(
            parsed.render(),
            r#"{"a":1.5,"b":2e3,"c":99999999999999999999}"#
        );
    }

    #[test]
    fn deep_nesting_is_refused_rather_than_overflowing_the_stack() {
        let deep = "[".repeat(500) + &"]".repeat(500);
        let err = parse(&deep).expect_err("a bounded refusal, not a crash");
        assert_eq!(err.wanted, "less deeply nested json");
    }

    #[test]
    fn a_raw_control_byte_inside_a_string_is_refused() {
        assert!(parse("{\"a\":\"b\u{1}c\"}").is_err());
    }

    #[test]
    fn a_parse_error_says_where_it_gave_up_and_what_it_wanted() {
        let err = parse(r#"{"a":1} {"b":2}"#).expect_err("two values");
        assert_eq!(
            err.to_string(),
            "invalid json at byte 8: expected end of input"
        );
    }

    #[test]
    fn displaying_a_value_renders_it() {
        assert_eq!(Value::obj([("a", Value::Num(1))]).to_string(), r#"{"a":1}"#);
    }

    #[test]
    fn get_on_a_non_object_is_none_rather_than_a_panic() {
        assert_eq!(Value::Num(1).get("a"), None);
        assert_eq!(Value::Num(1).as_str(), None);
    }
}
