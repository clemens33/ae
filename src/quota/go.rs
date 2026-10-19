//! Parser for `OpenCode` Go's usage answer and for ae's own cache record of it.
//!
//! The endpoint is undocumented, so its answer is hostile bytes: every shape
//! this parser was not measured against yields no number, only a named note.
//! The cache record is ae's own file under the state root, and it is hostile
//! too, because a human can edit it. Nothing here reads a file, the network or
//! a clock: the caller passes the instant the answer completed and the instant
//! it judges at.

use crate::json::{self, Value};
use crate::time::Timestamp;

use super::{Row, Status, freshness, percent};

/// The three windows the answer carries, with the length the vendor states.
///
/// `rolling` is the plan's 5-hour window and `weekly` is 7 days by name; a
/// month has no fixed length, so `monthly` states none and never counts as a
/// window proven at least a day long.
const WINDOWS: [(&str, Option<u32>); 3] = [
    ("rolling", Some(300)),
    ("weekly", Some(10_080)),
    ("monthly", None),
];

/// Most samples one cache record may carry.
pub const SAMPLES_MAX: usize = 48;

/// What one answer supplies: its windows, and why any window is missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    /// One row per readable window, or one `read-error` placeholder when none is.
    pub rows: Vec<Row>,
    /// Fixed-text notes; vendor text never reaches one.
    pub notes: Vec<String>,
}

impl Parsed {
    /// Whether at least one window was readable.
    #[must_use]
    pub fn usable(&self) -> bool {
        self.rows.iter().any(|row| row.status != Status::ReadError)
    }
}

/// Read one usage answer observed at `observed_at`, judged at `now`.
#[must_use]
pub fn parse(bytes: &[u8], observed_at: i64, now: i64) -> Parsed {
    let usage = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| json::parse(text).ok())
        .and_then(|root| match root.get("usage") {
            Some(usage @ Value::Obj(_)) => Some(usage.clone()),
            _ => None,
        });
    let Some(usage) = usage else {
        return unreadable("opencode go usage: the answer is not the measured shape".to_owned());
    };
    let mut parsed = Parsed::default();
    for (bucket, minutes) in WINDOWS {
        match window(usage.get(bucket), bucket, minutes, observed_at, now) {
            Some((row, note)) => {
                parsed.rows.push(row);
                parsed.notes.extend(note);
            }
            None => parsed
                .notes
                .push(format!("opencode go {bucket}: window unreadable")),
        }
    }
    if parsed.rows.is_empty() {
        let mut empty = unreadable("opencode go usage: no window was readable".to_owned());
        empty.notes.splice(0..0, parsed.notes);
        return empty;
    }
    parsed
}

/// The one `read-error` placeholder and the note saying why.
fn unreadable(note: String) -> Parsed {
    Parsed {
        rows: vec![Row {
            bucket: "-".to_owned(),
            qualifier: None,
            window_minutes: None,
            used_percent: None,
            resets_at: None,
            observed_at: None,
            status: Status::ReadError,
        }],
        notes: vec![note],
    }
}

/// One window, or `None` when anything in it is not the measured shape.
fn window(
    value: Option<&Value>,
    bucket: &str,
    minutes: Option<u32>,
    observed_at: i64,
    now: i64,
) -> Option<(Row, Option<String>)> {
    let limited = match value?.get_str("status")? {
        "ok" => false,
        "rate-limited" => true,
        _ => return None,
    };
    let literal = percent(value?.get("percent"))?;
    let resets_at = timestamp(value?.get_str("resetsAt")?)?;
    // `rate-limited` is the vendor's own verdict that the window is spent, so
    // the window reads at least full whatever its percentage says.
    let below_full = literal.parse::<f64>().is_ok_and(|used| used < 100.0);
    let used = if limited && below_full {
        "100".to_owned()
    } else {
        literal
    };
    let note = limited.then(|| format!("opencode go {bucket}: vendor says rate-limited"));
    Some((
        Row {
            bucket: bucket.to_owned(),
            qualifier: None,
            window_minutes: minutes,
            used_percent: Some(used),
            resets_at: Some(resets_at),
            observed_at: Some(observed_at),
            status: freshness(Some(observed_at), Some(resets_at), now),
        },
        note,
    ))
}

/// An RFC 3339 instant with `Z` or a `±hh:mm` offset, as epoch seconds.
#[must_use]
pub fn timestamp(text: &str) -> Option<i64> {
    Timestamp::parse_micros(text).map(|micros| micros.div_euclid(1_000_000))
}

/// The identity of the key file a cache record was written for: never its
/// content, and never a digest of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyIdentity {
    /// The canonical key path.
    pub path: String,
    /// Device of the key file.
    pub dev: u64,
    /// Inode of the key file.
    pub ino: u64,
    /// Length of the key file in bytes.
    pub len: u64,
    /// Modification time of the key file, in nanoseconds.
    pub mtime_ns: i64,
}

/// One good answer kept for pace, with the instant it completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// When the answer completed.
    pub at: i64,
    /// The vendor's own answer, verbatim.
    pub body: String,
}

/// The shared cache record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cached {
    /// The key file this record was written for.
    pub key: KeyIdentity,
    /// When the last attempt was made, whatever it answered.
    pub attempt_at: i64,
    /// The last attempt's outcome slug.
    pub outcome: String,
    /// The good answers kept, oldest first.
    pub samples: Vec<Sample>,
}

impl Cached {
    /// The record, rendered for the cache file.
    #[must_use]
    pub fn render(&self) -> String {
        Value::obj([
            ("v", Value::Num(1)),
            (
                "key",
                Value::obj([
                    ("path", Value::str(self.key.path.clone())),
                    ("dev", unsigned(self.key.dev)),
                    ("ino", unsigned(self.key.ino)),
                    ("len", unsigned(self.key.len)),
                    ("mtime_ns", Value::Num(self.key.mtime_ns)),
                ]),
            ),
            ("attempt_at", Value::Num(self.attempt_at)),
            ("outcome", Value::str(self.outcome.clone())),
            (
                "samples",
                Value::Arr(
                    self.samples
                        .iter()
                        .map(|sample| {
                            Value::obj([
                                ("at", Value::Num(sample.at)),
                                ("body", Value::str(sample.body.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
        .render()
    }
}

/// A `u64` as JSON: a number while it fits one, else its decimal literal.
fn unsigned(value: u64) -> Value {
    i64::try_from(value).map_or_else(|_| Value::Raw(value.to_string()), Value::Num)
}

/// Read a cache record, or `None` for anything ae did not write.
#[must_use]
pub fn parse_cache(bytes: &[u8]) -> Option<Cached> {
    let root = json::parse(std::str::from_utf8(bytes).ok()?).ok()?;
    if !matches!(root.get("v"), Some(Value::Num(1))) {
        return None;
    }
    let key = root.get("key")?;
    let key = KeyIdentity {
        path: key.get_str("path")?.to_owned(),
        dev: read_unsigned(key.get("dev")?)?,
        ino: read_unsigned(key.get("ino")?)?,
        len: read_unsigned(key.get("len")?)?,
        mtime_ns: match key.get("mtime_ns")? {
            Value::Num(number) => *number,
            _ => return None,
        },
    };
    let Value::Num(attempt_at) = root.get("attempt_at")? else {
        return None;
    };
    let outcome = root.get_str("outcome")?;
    if !is_outcome(outcome) {
        return None;
    }
    let Value::Arr(entries) = root.get("samples")? else {
        return None;
    };
    if entries.len() > SAMPLES_MAX {
        return None;
    }
    let mut samples = Vec::with_capacity(entries.len());
    for entry in entries {
        let Value::Num(at) = entry.get("at")? else {
            return None;
        };
        samples.push(Sample {
            at: *at,
            body: entry.get_str("body")?.to_owned(),
        });
    }
    Some(Cached {
        key,
        attempt_at: *attempt_at,
        outcome: outcome.to_owned(),
        samples,
    })
}

fn read_unsigned(value: &Value) -> Option<u64> {
    match value {
        Value::Num(number) => u64::try_from(*number).ok(),
        Value::Raw(raw) if raw.bytes().all(|byte| byte.is_ascii_digit()) => raw.parse().ok(),
        _ => None,
    }
}

/// The fixed outcome slugs, plus `status-NNN` for any other HTTP status.
const OUTCOMES: [&str; 9] = [
    "ok",
    "unauthorized",
    "not-entitled",
    "throttled",
    "timeout",
    "transport",
    "too-large",
    "malformed",
    "redirected",
];

/// Whether `slug` is an outcome ae writes.
#[must_use]
pub fn is_outcome(slug: &str) -> bool {
    OUTCOMES.contains(&slug)
        || slug
            .strip_prefix("status-")
            .is_some_and(|code| code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> KeyIdentity {
        KeyIdentity {
            path: "/home/u/.config/opencode/go-usage.key".to_owned(),
            dev: 1,
            ino: u64::MAX,
            len: 28,
            mtime_ns: 1_792_416_000_000_000_000,
        }
    }

    #[test]
    fn a_rendered_record_reads_back_whole() {
        let cached = Cached {
            key: identity(),
            attempt_at: 1_792_416_000,
            outcome: "status-502".to_owned(),
            samples: vec![Sample {
                at: 1_792_415_000,
                body: "{\"usage\":{}}\n\"quoted\"".to_owned(),
            }],
        };
        assert_eq!(parse_cache(cached.render().as_bytes()), Some(cached));
    }

    #[test]
    fn a_record_ae_never_writes_is_refused_whole() {
        let good = Cached {
            key: identity(),
            attempt_at: 5,
            outcome: "ok".to_owned(),
            samples: Vec::new(),
        }
        .render();
        for bad in [
            good.replace("\"v\":1", "\"v\":2"),
            good.replace("\"ok\"", "\"fine\""),
            good.replace("\"attempt_at\":5", "\"attempt_at\":\"5\""),
            good.replace("\"dev\":1", "\"dev\":-1"),
            "[]".to_owned(),
            "{".to_owned(),
        ] {
            assert_eq!(parse_cache(bad.as_bytes()), None, "{bad}");
        }
        assert!(is_outcome("status-418"));
        assert!(!is_outcome("status-4180"));
        assert!(!is_outcome("status-ab1"));
    }

    #[test]
    fn a_percent_at_or_over_full_keeps_its_literal_when_rate_limited() {
        let body = br#"{"usage":{"rolling":{"status":"rate-limited","percent":104.5,"resetsAt":"2026-10-19T17:00:00Z"}}}"#;
        let parsed = parse(body, 1_792_416_000, 1_792_416_000);
        assert_eq!(parsed.rows[0].used_percent.as_deref(), Some("104.5"));
    }

    #[test]
    fn a_negative_offset_moves_the_instant_later() {
        assert_eq!(
            timestamp("2026-10-19T10:00:00-02:30"),
            Timestamp::parse("2026-10-19T12:30:00Z").map(Timestamp::epoch)
        );
        assert_eq!(timestamp("2026-10-19T10:00:00+24:00"), None);
    }
}
