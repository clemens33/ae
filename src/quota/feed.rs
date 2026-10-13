//! Claude's quota as its statusline sees it — the payload a statusline hands
//! `ae _quota-feed` — and ae's own record of it. Both are hostile input, both
//! are pure bytes-to-values boundaries, and nothing here builds a level.

use std::fmt::Write as _;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt as _;
use std::path::PathBuf;

use crate::json;

use super::{ParseError, epoch, percent};

/// The most of a payload ae keeps; a longer one is drained and discarded whole.
pub const STDIN_MAX: usize = 64 * 1024;

/// The largest record the reader accepts and the writer may publish.
pub const RECORD_MAX: usize = 1024;

/// A record's scope path, in bytes.
pub const SCOPE_MAX: usize = 512;

/// Fractional digits a percentage may carry: the shortest spelling of the
/// vendor's value is kept exactly, and one needing more is not recorded.
pub const PCT_FRACTION_MAX: usize = 20;

/// How often an unchanged payload may refresh the record's observation time.
pub const REWRITE_SECS: i64 = 60;

/// The directory under the ae state root that holds one record per scope.
pub const DIR: &str = "quota-feed";

const HEADER: &str = "ae-quota-feed 1";

/// The latest second a four-digit year can name.
const EPOCH_MAX: i64 = 253_402_300_799;

/// Payload field, the `/usage` cache's bucket for it, minutes — record order.
const WINDOWS: [(&str, &str, u32); 2] = [
    ("five_hour", "session", 300),
    ("seven_day", "weekly_all", 10_080),
];

/// One window a payload stated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The `/usage` cache's bucket name for this window.
    pub bucket: &'static str,
    /// The window's length.
    pub window_minutes: u32,
    /// The used percentage, 0 to 100, in the shortest spelling of its value.
    pub used_percent: String,
    /// The window's reset, Unix epoch seconds.
    pub resets_at: i64,
}

/// What ae keeps for one Claude scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The canonical `.claude.json` path that names the scope.
    pub scope: PathBuf,
    /// When ae received the payload: the observation time of every window.
    pub received_at: i64,
    /// The windows the payload stated, in record order, at least one.
    pub windows: Vec<Window>,
}

/// Read `input` to its end, so the writer upstream never meets a closed pipe,
/// keeping at most [`STDIN_MAX`] bytes: `None` when over it or a read failed.
pub fn drain(input: &mut impl Read) -> Option<Vec<u8>> {
    let mut kept = Vec::new();
    let mut over = false;
    let mut chunk = [0_u8; 8192];
    loop {
        match input.read(&mut chunk) {
            Ok(0) => return (!over).then_some(kept),
            Ok(read) if !over && kept.len() + read <= STDIN_MAX => {
                kept.extend_from_slice(chunk.get(..read).unwrap_or_default());
            }
            Ok(_) => {
                over = true;
                kept = Vec::new();
            }
            Err(why) if why.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

/// The windows a payload states, each judged alone: one absent, null or
/// invalid is left out, and an empty answer means nothing to record.
#[must_use]
pub fn extract(payload: &[u8]) -> Vec<Window> {
    let root = std::str::from_utf8(payload)
        .ok()
        .and_then(|text| json::parse(text).ok());
    let Some(limits) = root.as_ref().and_then(|root| root.get("rate_limits")) else {
        return Vec::new();
    };
    WINDOWS
        .iter()
        .filter_map(|&(field, bucket, window_minutes)| {
            let window = limits.get(field)?;
            let used_percent = percent(window.get("used_percentage"))
                .and_then(|literal| literal.parse::<f64>().ok())
                .and_then(spelled)?;
            let resets_at =
                epoch(window.get("resets_at")).filter(|at| (1..=EPOCH_MAX).contains(at))?;
            Some(Window {
                bucket,
                window_minutes,
                used_percent,
                resets_at,
            })
        })
        .collect()
}

/// A percentage's one spelling: the shortest decimal round-tripping the value.
fn spelled(value: f64) -> Option<String> {
    if !(0.0..=100.0).contains(&value) {
        return None;
    }
    // `-0.0` is in range and would spell a sign.
    let text = format!("{}", value.abs());
    let digits = |part: &str, max: usize| {
        (1..=max).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit())
    };
    let spelled = match text.split_once('.') {
        Some((whole, fraction)) => digits(whole, 3) && digits(fraction, PCT_FRACTION_MAX),
        None => digits(&text, 3),
    };
    spelled.then_some(text)
}

/// FNV-1a 64 of `scope`; the record carries the scope, so a collision reads foreign.
#[must_use]
pub fn file_name(scope: &std::path::Path) -> String {
    let hash = crate::board::fnv_fold(crate::board::FNV_BASIS, scope.as_os_str().as_bytes());
    format!("claude-{hash:016x}")
}

/// Whether `incoming` is worth publishing over `held`: another scope or other
/// windows, or received [`REWRITE_SECS`] or more after it (or before it).
#[must_use]
pub fn due(held: &Record, incoming: &Record) -> bool {
    let age = incoming.received_at.saturating_sub(held.received_at);
    held.scope != incoming.scope
        || held.windows != incoming.windows
        || !(0..REWRITE_SECS).contains(&age)
}

/// The record's text, or `None` when [`parse`] would not give the same record
/// back: the writer never publishes what its reader refuses.
#[must_use]
pub fn render(record: &Record) -> Option<String> {
    let mut text = format!(
        "{HEADER}\nscope {}\nreceived {}\n",
        record.scope.to_str()?,
        record.received_at
    );
    for window in &record.windows {
        let (bucket, used, resets) = (window.bucket, &window.used_percent, window.resets_at);
        writeln!(text, "window {bucket} {used} {resets}").ok()?;
    }
    (parse(text.as_bytes()).as_ref() == Ok(record)).then_some(text)
}

/// Parse exactly the text [`render`] writes, and nothing else.
///
/// # Errors
///
/// [`ParseError::Utf8`] for bytes that are not UTF-8, else [`ParseError::Shape`].
pub fn parse(bytes: &[u8]) -> Result<Record, ParseError> {
    if bytes.len() > RECORD_MAX {
        return Err(ParseError::Shape);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ParseError::Utf8)?;
    let body = text.strip_suffix('\n').ok_or(ParseError::Shape)?;
    let mut lines = body.split('\n');
    let mut field = |name: &str| lines.next().and_then(|line| line.strip_prefix(name));
    let header = field(HEADER) == Some("");
    let scope = field("scope ").filter(|scope| {
        scope.starts_with('/') && scope.len() <= SCOPE_MAX && !scope.chars().any(char::is_control)
    });
    let received_at = field("received ").and_then(epoch_text);
    let (true, Some(scope), Some(received_at)) = (header, scope, received_at) else {
        return Err(ParseError::Shape);
    };
    let mut windows: Vec<Window> = Vec::new();
    for line in lines {
        let words: Vec<&str> = line
            .strip_prefix("window ")
            .unwrap_or_default()
            .split(' ')
            .collect();
        let after = windows.last().map_or(0, |held| held.window_minutes);
        let known = WINDOWS
            .iter()
            .find(|(_, bucket, minutes)| words.first() == Some(bucket) && *minutes > after);
        let used = words
            .get(1)
            .filter(|used| used.parse().ok().and_then(spelled).as_deref() == Some(**used));
        let (Some(&(_, bucket, window_minutes)), Some(used), Some(resets_at), 3) = (
            known,
            used,
            words.get(2).and_then(|at| epoch_text(at)),
            words.len(),
        ) else {
            return Err(ParseError::Shape);
        };
        let used_percent = (*used).to_owned();
        windows.push(Window {
            bucket,
            window_minutes,
            used_percent,
            resets_at,
        });
    }
    let scope = PathBuf::from(scope);
    (!windows.is_empty())
        .then_some(Record {
            scope,
            received_at,
            windows,
        })
        .ok_or(ParseError::Shape)
}

/// A positive epoch in its one spelling: no sign, no leading zero.
fn epoch_text(text: &str) -> Option<i64> {
    let canonical = text.bytes().all(|byte| byte.is_ascii_digit()) && !text.starts_with('0');
    canonical
        .then(|| text.parse().ok())
        .flatten()
        .filter(|at| (1..=EPOCH_MAX).contains(at))
}

#[cfg(test)]
mod tests {
    use super::{PCT_FRACTION_MAX, Record, STDIN_MAX, Window, due, extract, parse, render};
    use std::io::Cursor;
    use std::path::PathBuf;

    const BOTH: &str = r#"{"rate_limits":{"five_hour":{"used_percentage":42,"resets_at":4070908800},"seven_day":{"used_percentage":84,"resets_at":4070908800}}}"#;

    fn windows(five: &str, seven: &str) -> Vec<Window> {
        extract(
            format!(r#"{{"rate_limits":{{"five_hour":{five},"seven_day":{seven}}}}}"#).as_bytes(),
        )
    }

    fn record(scope: &str) -> Record {
        Record {
            scope: PathBuf::from(scope),
            received_at: 1_000,
            windows: extract(BOTH.as_bytes()),
        }
    }

    #[test]
    fn the_drain_keeps_up_to_the_cap_and_consumes_every_byte_past_it() {
        let mut exact = Cursor::new(vec![b' '; STDIN_MAX]);
        assert_eq!(
            super::drain(&mut exact).map(|kept| kept.len()),
            Some(STDIN_MAX)
        );
        // The SAME valid payload: under the cap it records, one byte over it
        // records nothing, and either way the input is read to its end.
        let mut padded = BOTH.as_bytes().to_vec();
        padded.resize(STDIN_MAX, b' ');
        assert_eq!(
            extract(&super::drain(&mut Cursor::new(padded.clone())).unwrap_or_default()).len(),
            2
        );
        padded.push(b' ');
        let mut over = Cursor::new(padded);
        assert_eq!(super::drain(&mut over), None);
        assert_eq!(
            over.position(),
            u64::try_from(STDIN_MAX + 1).unwrap_or(u64::MAX)
        );
    }

    #[test]
    fn each_window_is_judged_alone_and_an_empty_answer_records_nothing() {
        let valid = r#"{"used_percentage":42,"resets_at":4070908800}"#;
        let kept = windows(valid, r#"{"used_percentage":"84","resets_at":4070908800}"#);
        assert_eq!(
            kept.iter().map(|window| window.bucket).collect::<Vec<_>>(),
            ["session"]
        );
        let kept = windows("null", valid);
        assert_eq!(
            kept.iter()
                .map(|window| (window.bucket, window.window_minutes))
                .collect::<Vec<_>>(),
            [("weekly_all", 10_080)]
        );
        for payload in [
            r#"{"rate_limits":null}"#,
            r#"{"rate_limits":[]}"#,
            "{}",
            "",
            "\u{0}",
        ] {
            assert!(extract(payload.as_bytes()).is_empty(), "{payload:?}");
        }
    }

    #[test]
    fn a_percentage_keeps_the_vendor_value_exactly_inside_its_bounds() {
        let spelled = |used: &str| {
            windows(
                &format!(r#"{{"used_percentage":{used},"resets_at":4070908800}}"#),
                "null",
            )
            .first()
            .map(|window| window.used_percent.clone())
        };
        for (used, kept) in [
            ("79.999", "79.999"),
            ("42.50", "42.5"),
            ("-0.0", "0"),
            ("100", "100"),
            ("1e-7", "0.0000001"),
            ("2e1", "20"),
        ] {
            assert_eq!(spelled(used).as_deref(), Some(kept), "{used}");
        }
        let longest = format!("0.{}1", "0".repeat(PCT_FRACTION_MAX - 1));
        assert_eq!(spelled(&longest).as_deref(), Some(longest.as_str()));
        for used in ["100.0001", "-1", "1e-30", "\"42\"", "null", "1e400"] {
            assert_eq!(spelled(used), None, "{used}");
        }
        assert!(windows(r#"{"used_percentage":1,"resets_at":1.5}"#, "null").is_empty());
        assert!(windows(r#"{"used_percentage":1,"resets_at":0}"#, "null").is_empty());
    }

    #[test]
    fn a_record_round_trips_and_every_other_spelling_is_refused() {
        let held = record("/home/u/.claude.json");
        let text = render(&held).unwrap_or_default();
        assert_eq!(
            text,
            "ae-quota-feed 1\nscope /home/u/.claude.json\nreceived 1000\nwindow session 42 4070908800\nwindow weekly_all 84 4070908800\n"
        );
        assert_eq!(parse(text.as_bytes()), Ok(held));
        for (from, to) in [
            ("received 1000", "received 01000"),
            ("received 1000", "received +1000"),
            ("session 42 ", "session 042 "),
            ("session 42 ", "session 42.0 "),
            ("session 42 ", "session 42  "),
            ("4070908800\nwindow", "4070908800 \nwindow"),
            (
                "window session 42 4070908800\nwindow weekly_all 84 4070908800\n",
                "window weekly_all 84 4070908800\nwindow session 42 4070908800\n",
            ),
            (
                "window weekly_all 84 4070908800\n",
                "window session 84 4070908800\n",
            ),
            ("window session", "window weekly_scoped"),
            ("scope /home", "scope home"),
            ("scope /home", "scope /h\u{7}ome"),
            ("1000\n", "1000\r\n"),
        ] {
            assert!(
                parse(text.replacen(from, to, 1).as_bytes()).is_err(),
                "{from:?} -> {to:?}"
            );
        }
        let only = "ae-quota-feed 1\nscope /a\nreceived 1\n";
        assert!(parse(only.as_bytes()).is_err());
        assert!(parse(text.trim_end().as_bytes()).is_err());
        assert!(parse(format!("{text}window weekly_all 84 4070908800\n").as_bytes()).is_err());
        assert!(parse(b"ae-quota-feed 1\nscope /\xff\nreceived 1\nwindow session 1 1\n").is_err());
    }

    #[test]
    fn the_writer_publishes_nothing_its_own_reader_would_refuse() {
        let longest = format!("/{}", "a".repeat(super::SCOPE_MAX - 1));
        assert!(render(&record(&longest)).is_some());
        for scope in [
            format!("{longest}b"),
            "relative".to_owned(),
            "/a\nb".to_owned(),
        ] {
            assert_eq!(render(&record(&scope)), None, "{scope:?}");
        }
        let mut odd = record("/a");
        if let Some(window) = odd.windows.first_mut() {
            window.used_percent = "042".to_owned();
        }
        assert_eq!(render(&odd), None);
    }

    #[test]
    fn an_unchanged_record_is_due_again_only_after_sixty_seconds() {
        let held = record("/a");
        let at = |received_at: i64| Record {
            received_at,
            ..held.clone()
        };
        assert!(!due(&held, &at(1_000)));
        assert!(!due(&held, &at(1_059)));
        assert!(due(&held, &at(1_060)));
        assert!(due(&held, &at(999)));
        let mut moved = at(1_001);
        moved.windows.pop();
        assert!(due(&held, &moved));
        assert!(due(
            &held,
            &Record {
                scope: PathBuf::from("/b"),
                ..at(1_001)
            }
        ));
    }
}
