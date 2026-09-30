//! Parser for agy's on-demand `/quota` report and its `--version` line.
//!
//! Both are the vendor's stdout, read back from a bounded scratch capture, so
//! both are hostile bytes. Nothing here runs a process or reads a clock: the
//! caller passes the instant the call completed.

use crate::json::{self, Value};

use super::{ParseError, Row, Status, freshness, vendor_timestamp};

/// The first agy whose print mode answers `/quota` without starting an agent
/// turn (vendor changelog 1.1.11). An older one reads `/quota` as a prompt.
pub const FLOOR: Version = Version {
    major: 1,
    minor: 1,
    patch: 11,
    pre: false,
};

/// Longest pre-release or build-metadata identifier accepted after the triple.
const SUFFIX_MAX: usize = 32;
/// Most groups one report may carry; 1.2.14 reports two.
const MAX_GROUPS: usize = 8;
/// Most buckets one group may carry; 1.2.14 reports two per group.
const MAX_BUCKETS: usize = 16;
/// Longest bucket id or group name a cell may show.
const NAME_MAX: usize = 40;

/// One `agy --version` answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    /// Major release.
    pub major: u32,
    /// Minor release.
    pub minor: u32,
    /// Patch release.
    pub patch: u32,
    /// A `-<id>` pre-release suffix, which sorts BEFORE its bare triple.
    pub pre: bool,
}

impl Version {
    /// Whether this release answers `/quota` without a turn.
    ///
    /// The triple is compared as integers, never as text (`1.1.9 < 1.1.11`),
    /// and a pre-release of the floor itself is below it, as semver orders it:
    /// `1.1.11-rc1` is refused, `1.1.11+b7` and `1.1.12-rc1` pass.
    #[must_use]
    pub fn passes_floor(self) -> bool {
        let triple = (self.major, self.minor, self.patch);
        let floor = (FLOOR.major, FLOOR.minor, FLOOR.patch);
        triple > floor || (triple == floor && !self.pre)
    }
}

/// Why a `/quota` report cannot supply rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The bytes are not a report of the measured shape.
    Parse(ParseError),
    /// The report says an agent turn ran, so the call may have spent quota.
    SpentTurn,
}

/// Read one `agy --version` answer, strictly: `X.Y.Z`, an optional `-<id>` or
/// `+<id>` suffix, and at most one trailing newline — nothing else. A prefix,
/// a second line, or any other shape is `None`, which closes the version gate.
#[must_use]
pub fn version(bytes: &[u8]) -> Option<Version> {
    let text = std::str::from_utf8(bytes).ok()?;
    let line = text.strip_suffix('\n').unwrap_or(text);
    let (triple, pre) = match line.find(['-', '+']) {
        Some(at) => {
            let (triple, suffix) = line.split_at(at);
            let id = suffix.get(1..)?;
            let identifier = !id.is_empty()
                && id.len() <= SUFFIX_MAX
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-');
            if !identifier {
                return None;
            }
            (triple, suffix.starts_with('-'))
        }
        None => (line, false),
    };
    let mut parts = triple.split('.');
    let found = Version {
        major: release_number(parts.next()?)?,
        minor: release_number(parts.next()?)?,
        patch: release_number(parts.next()?)?,
        pre,
    };
    parts.next().is_none().then_some(found)
}

/// One component of the triple: 1..=9 ASCII digits, so it always fits a u32.
fn release_number(text: &str) -> Option<u32> {
    if text.is_empty() || text.len() > 9 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Whether `bytes` is a version answer at or above [`FLOOR`].
#[must_use]
pub fn supports_quota(bytes: &[u8]) -> bool {
    version(bytes).is_some_and(Version::passes_floor)
}

/// Parse one complete `agy -p /quota --output-format json` report into one row
/// per bucket, each observed at `now`, the instant the call completed.
///
/// # Errors
///
/// [`Refusal::SpentTurn`] when the report names any agent turn, and
/// [`Refusal::Parse`] for invalid UTF-8, malformed JSON, a key repeated in any
/// object the report is read from, or a report that is not a successful
/// `usage` answer of the measured shape.
pub fn parse(bytes: &[u8], now: i64) -> Result<Vec<Row>, Refusal> {
    const SHAPE: Refusal = Refusal::Parse(ParseError::Shape);
    let text = std::str::from_utf8(bytes).map_err(|_| Refusal::Parse(ParseError::Utf8))?;
    let parsed = json::parse(text).map_err(|_| Refusal::Parse(ParseError::Json))?;
    let root = distinct(&parsed).ok_or(SHAPE)?;
    // Before any other field, so a turn in a root with no repeated key is
    // named whatever else the report says: the call exists to spend nothing,
    // and a report that says otherwise supplies no quota.
    if root
        .get("num_turns")
        .is_some_and(|turns| *turns != Value::Num(0))
    {
        return Err(Refusal::SpentTurn);
    }
    let command = root.get("command").and_then(distinct).ok_or(SHAPE)?;
    if root.get_str("status") != Some("SUCCESS") || command.get_str("name") != Some("usage") {
        return Err(SHAPE);
    }
    let data = command.get("data").and_then(distinct).ok_or(SHAPE)?;
    let Some(Value::Arr(groups)) = data.get("groups") else {
        return Err(SHAPE);
    };
    if groups.len() > MAX_GROUPS {
        return Err(SHAPE);
    }
    let mut rows = Vec::new();
    for group in groups {
        let group = distinct(group).ok_or(SHAPE)?;
        let Some(Value::Arr(buckets)) = group.get("buckets") else {
            return Err(SHAPE);
        };
        if buckets.len() > MAX_BUCKETS {
            return Err(SHAPE);
        }
        let qualifier = group
            .get_str("name")
            .filter(|name| is_family_name(name))
            .map(str::to_owned);
        for bucket in buckets {
            rows.push(bucket_row(bucket, qualifier.clone(), now).ok_or(SHAPE)?);
        }
    }
    rows.sort_by(|left, right| {
        left.qualifier
            .cmp(&right.qualifier)
            .then_with(|| left.bucket.cmp(&right.bucket))
    });
    Ok(rows)
}

/// One bucket's row, or `None` when the bucket names no provable id: a row ae
/// cannot name is not guessed at, and the whole report is refused instead.
fn bucket_row(bucket: &Value, qualifier: Option<String>, now: i64) -> Option<Row> {
    let bucket = distinct(bucket)?;
    let id = bucket.get_str("id").filter(|id| is_bucket_id(id))?;
    let window_minutes = match bucket.get_str("window") {
        Some("5h") => Some(300),
        Some("weekly") => Some(10_080),
        _ => None,
    };
    let (used_percent, resets_at) = if window_minutes.is_some() {
        (
            used_percent(bucket.get("remaining_fraction")),
            bucket.get_str("reset_time").and_then(vendor_timestamp),
        )
    } else {
        (None, None)
    };
    let status = if window_minutes.is_some() && used_percent.is_some() {
        freshness(Some(now), resets_at, now)
    } else {
        Status::Unknown
    };
    Some(Row {
        bucket: id.to_owned(),
        qualifier,
        window_minutes,
        used_percent,
        resets_at,
        observed_at: Some(now),
        status,
    })
}

/// `value` when it is an object whose keys are all different, else `None`.
///
/// `json::parse` keeps every member and `get` answers the first, so a repeated
/// key would let one report say two things — `num_turns` 0 and then 1 among
/// them. Every object this parser reads passes here, and a repeat refuses the
/// whole report rather than choosing one of its answers.
fn distinct(value: &Value) -> Option<&Value> {
    let Value::Obj(fields) = value else {
        return None;
    };
    let mut keys: Vec<&str> = fields.iter().map(|(key, _)| key.as_str()).collect();
    keys.sort_unstable();
    keys.windows(2)
        .all(|pair| pair[0] != pair[1])
        .then_some(value)
}

/// The USED percentage, from agy's REMAINING fraction: a number in 0..=1, to
/// one decimal. Anything else — a string, a negative, past one, infinite — is
/// no number at all.
fn used_percent(value: Option<&Value>) -> Option<String> {
    let remaining = match value? {
        Value::Num(0) => 0.0,
        Value::Num(1) => 1.0,
        Value::Raw(raw) => raw.parse::<f64>().ok()?,
        _ => return None,
    };
    (0.0..=1.0)
        .contains(&remaining)
        .then(|| format!("{:.1}", 100.0 * (1.0 - remaining)))
}

/// A bucket id ae may print: `[A-Za-z0-9][A-Za-z0-9._-]{0,39}`.
fn is_bucket_id(text: &str) -> bool {
    is_name(text, |byte| matches!(byte, b'.' | b'_' | b'-'))
}

/// A group name ae may print: `[A-Za-z0-9][A-Za-z0-9 ._&+()/-]{0,39}`.
fn is_family_name(text: &str) -> bool {
    is_name(text, |byte| {
        matches!(
            byte,
            b' ' | b'.' | b'_' | b'&' | b'+' | b'(' | b')' | b'/' | b'-'
        )
    })
}

fn is_name(text: &str, punctuation: impl Fn(u8) -> bool) -> bool {
    let bytes = text.as_bytes();
    bytes.len() <= NAME_MAX
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || punctuation(*byte))
}

#[cfg(test)]
mod tests {
    use super::{Refusal, parse, supports_quota, version};
    use crate::quota::{ParseError, Status};
    use crate::time::Timestamp;

    /// The one real `agy -p /quota --output-format json` answer (agy 1.2.14,
    /// 2026-10-01), byte-exact; it names no account.
    const MEASURED: &[u8] = include_bytes!("../../tests/fixtures/quota/agy-quota-1.2.14.json");

    fn epoch(text: &str) -> i64 {
        Timestamp::parse(text).expect("fixture timestamp").epoch()
    }

    fn report(groups: &str) -> String {
        format!(
            r#"{{"status":"SUCCESS","num_turns":0,"command":{{"name":"usage","data":{{"groups":{groups}}}}}}}"#
        )
    }

    fn one_bucket(bucket: &str) -> String {
        report(&format!(
            r#"[{{"name":"Gemini Models","buckets":[{bucket}]}}]"#
        ))
    }

    #[test]
    fn the_measured_report_yields_its_four_windows() {
        let now = epoch("2026-10-01T00:00:00Z");
        let rows = parse(MEASURED, now).expect("the measured report parses");
        let seen: Vec<_> = rows
            .iter()
            .map(|row| {
                format!(
                    "{:?} {} {:?} {:?}",
                    row.qualifier, row.bucket, row.window_minutes, row.used_percent
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                r#"Some("Claude and GPT models") 3p-5h Some(300) Some("0.0")"#,
                r#"Some("Claude and GPT models") 3p-weekly Some(10080) Some("0.0")"#,
                r#"Some("Gemini Models") gemini-5h Some(300) Some("0.0")"#,
                r#"Some("Gemini Models") gemini-weekly Some(10080) Some("4.0")"#,
            ]
        );
        assert_eq!(rows[3].resets_at, Some(epoch("2026-10-07T05:49:23Z")));
        assert_eq!(rows[0].resets_at, Some(epoch("2026-10-01T03:45:46Z")));
        assert!(rows.iter().all(|row| row.observed_at == Some(now)));
        assert!(rows.iter().all(|row| row.status == Status::Fresh));
    }

    #[test]
    fn used_is_derived_from_remaining_and_refused_outside_zero_to_one() {
        // One row per branch: the two integer literals, a float inside, each
        // bound of the range, and values of other JSON types.
        for (fraction, used) in [
            ("0.75", Some("25.0")),
            ("1", Some("0.0")),
            ("0", Some("100.0")),
            ("1.5", None),
            ("-0.1", None),
            ("\"0.5\"", None),
            ("null", None),
        ] {
            let text = one_bucket(&format!(
                r#"{{"id":"gemini-5h","window":"5h","remaining_fraction":{fraction},"reset_time":"2099-01-01T00:00:00Z"}}"#
            ));
            let rows = parse(text.as_bytes(), 0).expect("a well-formed report");
            assert_eq!(rows[0].used_percent.as_deref(), used, "{fraction}");
            let expected = if used.is_some() {
                Status::Fresh
            } else {
                Status::Unknown
            };
            assert_eq!(rows[0].status, expected, "{fraction}");
        }
    }

    #[test]
    fn an_unknown_window_keeps_its_row_but_no_numbers() {
        let text = one_bucket(
            r#"{"id":"gemini-day","window":"daily","remaining_fraction":0.5,"reset_time":"2099-01-01T00:00:00Z"}"#,
        );
        let rows = parse(text.as_bytes(), 0).expect("a well-formed report");
        assert_eq!(rows[0].bucket, "gemini-day");
        assert_eq!(rows[0].window_minutes, None);
        assert_eq!(rows[0].used_percent, None);
        assert_eq!(rows[0].resets_at, None);
        assert_eq!(rows[0].status, Status::Unknown);
    }

    #[test]
    fn a_passed_or_missing_reset_is_unknown() {
        for reset in [r#""2000-01-01T00:00:00Z""#, r#""soon""#] {
            let text = one_bucket(&format!(
                r#"{{"id":"gemini-5h","window":"5h","remaining_fraction":1,"reset_time":{reset}}}"#
            ));
            let rows = parse(text.as_bytes(), epoch("2026-10-01T00:00:00Z")).expect("parses");
            assert_eq!(rows[0].status, Status::Unknown, "{reset}");
        }
    }

    #[test]
    fn any_reported_turn_is_refused_even_on_an_odd_report() {
        // Only the integer 0 is no turn; a string "0" is not the same answer.
        let text = report("[]").replace(r#""num_turns":0"#, r#""num_turns":"0""#);
        assert_eq!(parse(text.as_bytes(), 0), Err(Refusal::SpentTurn));
        let failed = r#"{"status":"ERROR","num_turns":2}"#;
        assert_eq!(parse(failed.as_bytes(), 0), Err(Refusal::SpentTurn));
        // A second answer cannot hide behind the first one `get` would read,
        // even with another key between the two.
        let second = r#""num_turns":0,"duration_seconds":0,"num_turns":1"#;
        let hidden = report("[]").replace(r#""num_turns":0"#, second);
        assert_eq!(
            parse(hidden.as_bytes(), 0),
            Err(Refusal::Parse(ParseError::Shape))
        );
        // Absent is not a turn: the measured shape is the only proof asked for.
        let absent = report("[]").replace(r#""num_turns":0,"#, "");
        assert_eq!(parse(absent.as_bytes(), 0), Ok(Vec::new()));
    }

    #[test]
    fn an_empty_successful_report_is_no_rows_not_a_refusal() {
        assert_eq!(parse(report("[]").as_bytes(), 0), Ok(Vec::new()));
    }

    #[test]
    fn a_report_of_another_shape_is_refused_whole() {
        let shape = Err(Refusal::Parse(ParseError::Shape));
        let bucket = r#"{"id":"gemini-5h","window":"5h","remaining_fraction":1,"reset_time":"2099-01-01T00:00:00Z"}"#;
        let many_groups = format!("[{}]", [r#"{"name":"g","buckets":[]}"#; 9].join(","));
        let many_buckets = report(&format!(
            r#"[{{"name":"g","buckets":[{}]}}]"#,
            [bucket; 17].join(",")
        ));
        // One row per refusing branch, in the order `parse` meets them.
        for text in [
            "[]".to_owned(),
            report("[]").replace(r#","command":{"name":"usage","data":{"groups":[]}}"#, ""),
            report("[]").replace("SUCCESS", "ERROR"),
            report("[]").replace(r#""name":"usage""#, r#""name":"credits""#),
            report("{}"),
            report(&many_groups),
            report(r#"[{"name":"g"}]"#),
            many_buckets,
            one_bucket(&bucket.replace(r#""id":"gemini-5h","#, "")),
            one_bucket(&bucket.replace("gemini-5h", &"g".repeat(41))),
            one_bucket(&bucket.replace("gemini-5h", "")),
            one_bucket(&bucket.replace("gemini-5h", "-gemini-5h")),
            one_bucket(&bucket.replace("gemini-5h", "gemini 5h")),
            one_bucket(&bucket.replace("gemini-5h", "gemini-\\u001b[31m")),
            // A repeated key in each object the report is read from; at the
            // root even an agreeing `num_turns` is one answer too many.
            report("[]").replace(r#""num_turns":0"#, r#""num_turns":0,"num_turns":0"#),
            report("[]").replace(r#""name":"usage""#, r#""name":"usage","name":"usage""#),
            report("[]").replace(r#""groups":[]"#, r#""groups":[],"groups":[]"#),
            report(r#"[{"name":"g","buckets":[],"buckets":[]}]"#),
            one_bucket(&bucket.replace(r#""id":"gemini-5h","#, r#""id":"a","id":"b","#)),
        ] {
            assert_eq!(parse(text.as_bytes(), 0), shape, "{text}");
        }
        assert_eq!(
            parse(b"{\"status\":\"SUCCESS\xff\"}", 0),
            Err(Refusal::Parse(ParseError::Utf8))
        );
        // The measured TSV spelling is not the JSON report.
        let tsv = b"Gemini Models\tWeekly Limit Remaining\t96%\t2026-10-07T05:49:23Z\n";
        assert_eq!(parse(tsv, 0), Err(Refusal::Parse(ParseError::Json)));
    }

    #[test]
    fn the_largest_report_of_the_measured_shape_is_read_whole() {
        let bucket = |at: usize| {
            format!(
                r#"{{"id":"{}{at:02}","window":"5h","remaining_fraction":1,"reset_time":"2099-01-01T00:00:00Z"}}"#,
                "b".repeat(38)
            )
        };
        let group = |at: usize| {
            format!(
                r#"{{"name":"{}{at}","buckets":[{}]}}"#,
                "G".repeat(39),
                (0..16).map(bucket).collect::<Vec<_>>().join(",")
            )
        };
        let text = report(&format!(
            "[{}]",
            (0..8).map(group).collect::<Vec<_>>().join(",")
        ));
        let rows = parse(text.as_bytes(), 0).expect("eight groups of sixteen is in bounds");
        assert_eq!(rows.len(), 128);
        assert_eq!(rows[0].bucket.len(), 40);
        assert_eq!(rows[0].qualifier.as_deref().map(str::len), Some(40));
    }

    #[test]
    fn a_family_name_outside_the_allowlist_is_dropped_not_cleaned() {
        let bucket = r#"{"id":"gemini-5h","window":"5h","remaining_fraction":1,"reset_time":"2099-01-01T00:00:00Z"}"#;
        // Each way the allowlist refuses: a byte outside it, nothing at all, a
        // permitted byte where the first must be alphanumeric, and one
        // character past the width.
        for name in ["Gemini\u{1b}[2J", "", "-Gemini", "n".repeat(41).as_str()] {
            let text = report(&format!(
                r#"[{{"name":{},"buckets":[{bucket}]}}]"#,
                crate::json::Value::str(name).render()
            ));
            let rows = parse(text.as_bytes(), 0).expect("the row survives");
            assert_eq!(rows[0].qualifier, None, "{name:?}");
        }
        let text = report(&format!(r#"[{{"buckets":[{bucket}]}}]"#));
        assert_eq!(
            parse(text.as_bytes(), 0).expect("parses")[0].qualifier,
            None
        );
    }

    #[test]
    fn the_version_grammar_is_strict_and_the_floor_is_numeric() {
        // One row per branch of the grammar: with and without the newline,
        // each suffix kind, then each way a component or a suffix is refused.
        for (text, parsed) in [
            ("1.2.14\n", Some((1, 2, 14, false))),
            ("1.2.14", Some((1, 2, 14, false))),
            ("1.1.11-rc1\n", Some((1, 1, 11, true))),
            ("1.1.11+b7\n", Some((1, 1, 11, false))),
            ("release 1.2.14\n", None),
            ("1.2\n", None),
            ("1.2.14.1\n", None),
            ("1..14\n", None),
            ("1000000000.0.0\n", None),
            ("1.2.14-\n", None),
            ("1.2.14-rc_1\n", None),
        ] {
            let got = version(text.as_bytes()).map(|v| (v.major, v.minor, v.patch, v.pre));
            assert_eq!(got, parsed, "{text:?}");
        }
        assert_eq!(
            version(format!("1.2.3-{}\n", "a".repeat(33)).as_bytes()),
            None
        );
        assert!(version(format!("1.2.3-{}\n", "a".repeat(32)).as_bytes()).is_some());
        assert_eq!(version(&[b'1', b'.', b'2', b'.', 0xff]), None);

        // Each axis above and below the floor, compared as integers (`9 < 11`,
        // where text order says the opposite); the floor, its pre-release, a
        // pre-release above it, the suffix trick, and no version at all.
        for (text, passes) in [
            ("2.0.0\n", true),
            ("1.2.14\n", true),
            ("1.1.12-rc1\n", true),
            ("1.1.11\n", true),
            ("1.1.11-rc1\n", false),
            ("1.1.9-1.1.11\n", false),
            ("1.1.9\n", false),
            ("1.0.99\n", false),
            ("0.9.99\n", false),
            ("release 1.2.14\n", false),
        ] {
            assert_eq!(supports_quota(text.as_bytes()), passes, "{text:?}");
        }
    }

    #[test]
    fn ten_thousand_hostile_inputs_never_panic() {
        let mut state = 0x3c6e_f372_fe94_f82b_u64;
        for case in 0..10_000_usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = usize::try_from(state & 0xff).expect("bounded length");
            let mut bytes = Vec::with_capacity(len);
            for offset in 0..len {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                bytes.push(state.to_le_bytes()[offset & 7]);
            }
            let _ = parse(&bytes, i64::try_from(case).expect("bounded case"));
            let _ = supports_quota(&bytes);
        }
    }
}
