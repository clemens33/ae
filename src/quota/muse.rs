//! Parser for Muse's on-demand allowance and ae's own cache record of it.
//!
//! The allowance rides one `run.usage` record in `muse exec --json`'s stdout,
//! so it is the vendor's bytes and hostile; the cache record is ae's own file
//! under the state root, which a human can edit, so it is hostile too. Nothing
//! here runs a process, reads a file or a clock: the caller passes the instant
//! it judges at.
//!
//! The cache keeps the NORMALIZED allowance only — never the stream around it —
//! and the one reader of both is [`allowance_of`].

use crate::json::{self, Value};

use super::agy::distinct;
use super::{ParseError, Row, Status, epoch, freshness, percent};

/// How long one attempt, whatever it answered, stands for every ae process.
/// Longer than [`super::FRESH_SECS`] on purpose: a held answer reads stale
/// with the age the vendor observed it at, not the age of the hold.
pub const TTL_SECS: i64 = 1800;

/// The weekly allowance's length, from the vendor's own key name.
const WEEKLY_MINUTES: u32 = 10_080;
/// The latest vendor stamp or reset ae believes: 9999-12-31T23:59:59.999Z.
const MS_MAX: i64 = 253_402_300_799_999;
/// Longest tier name a cell may show.
const TIER_MAX: usize = 40;
/// The cache record's version.
const CACHE_VERSION: i64 = 1;

/// One window of the allowance, as the vendor stated it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Window {
    /// The used-percent literal, when it is a number ae can show.
    used: Option<String>,
    /// The reset instant in epoch milliseconds, when it is a clock.
    resets_ms: Option<i64>,
}

/// The allowance one `run.usage` record carries, reduced to what ae shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowance {
    tier: Option<String>,
    window: Option<Window>,
    weekly: Option<Window>,
    as_of_ms: Option<i64>,
}

impl Allowance {
    /// An allowance with no window: the record named none.
    const fn none() -> Self {
        Self {
            tier: None,
            window: None,
            weekly: None,
            as_of_ms: None,
        }
    }

    /// One row per window the record stated, judged at `now`. The short window
    /// states no length, and none is inferred from its reset; the weekly one is
    /// seven days by name. Both are observed when the VENDOR observed them, so a
    /// held answer keeps its age.
    #[must_use]
    pub fn rows(&self, now: i64) -> Vec<Row> {
        let observed_at = self.as_of_ms.map(|stamp| stamp.div_euclid(1000));
        [
            ("window", None, &self.window),
            ("weekly", Some(WEEKLY_MINUTES), &self.weekly),
        ]
        .into_iter()
        .filter_map(|(bucket, window_minutes, window)| {
            let window = window.as_ref()?;
            let resets_at = window.resets_ms.map(|reset| reset.div_euclid(1000));
            Some(Row {
                bucket: bucket.to_owned(),
                qualifier: self.tier.clone(),
                window_minutes,
                used_percent: window.used.clone(),
                resets_at,
                observed_at,
                status: if window.used.is_some() {
                    freshness(observed_at, resets_at, now)
                } else {
                    Status::Unknown
                },
            })
        })
        .collect()
    }

    fn value(&self) -> Value {
        let window = |held: &Window| {
            let mut fields = Vec::new();
            if let Some(used) = &held.used {
                fields.push(("used_percent", Value::Raw(used.clone())));
            }
            if let Some(reset) = held.resets_ms {
                fields.push(("resets_at_ms", Value::Num(reset)));
            }
            Value::obj(fields)
        };
        let mut fields = Vec::new();
        if let Some(tier) = &self.tier {
            fields.push(("tier", Value::str(tier.clone())));
        }
        if let Some(held) = &self.window {
            fields.push(("window", window(held)));
        }
        if let Some(held) = &self.weekly {
            fields.push(("weekly", window(held)));
        }
        if let Some(stamp) = self.as_of_ms {
            fields.push(("as_of_ms", Value::Num(stamp)));
        }
        Value::obj(fields)
    }
}

/// Read the allowance out of one complete `muse exec --json` stdout.
///
/// Only a line carrying the literal `"run.usage"` is looked at; a record of
/// `record_type` `status` and `payload_type` `run.usage` is a candidate, and
/// every other line is noise. A line that names `run.usage` and is not JSON, or
/// repeats a key in an object read from it, refuses the whole stream. With
/// several candidates the newest `as_of_ms` wins whatever its percentages say,
/// so an older number is never promoted; a candidate with no usable stamp among
/// others, or two newest candidates that differ, refuse the stream.
///
/// # Errors
///
/// [`ParseError::Utf8`] for invalid UTF-8, [`ParseError::Json`] for a
/// `run.usage` line that is not JSON, and [`ParseError::Shape`] for everything
/// else that is not the measured record.
pub fn extract(bytes: &[u8]) -> Result<Allowance, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ParseError::Utf8)?;
    let mut seen = Vec::new();
    let mut bare = false;
    for line in text.lines() {
        match candidate(line)? {
            Line::Noise => {}
            Line::Bare => bare = true,
            Line::Allowance(allowance) => seen.push(allowance),
        }
    }
    match seen.len() {
        0 if bare => Ok(Allowance::none()),
        0 => Err(ParseError::Shape),
        1 => seen.pop().ok_or(ParseError::Shape),
        _ => newest(seen),
    }
}

/// Parse one complete `muse exec --json` stdout into the rows judged at `now`.
///
/// # Errors
///
/// Whatever [`extract`] refuses.
pub fn parse(bytes: &[u8], now: i64) -> Result<Vec<Row>, ParseError> {
    extract(bytes).map(|allowance| allowance.rows(now))
}

/// What one stream line is.
enum Line {
    Noise,
    /// A `run.usage` record that states no allowance.
    Bare,
    Allowance(Allowance),
}

fn candidate(line: &str) -> Result<Line, ParseError> {
    if !line.contains("\"run.usage\"") {
        return Ok(Line::Noise);
    }
    let root = json::parse(line).map_err(|_| ParseError::Json)?;
    if !matches!(root, Value::Obj(_)) {
        return Ok(Line::Noise);
    }
    let root = distinct(&root).ok_or(ParseError::Shape)?;
    if root.get_str("record_type") != Some("status")
        || root.get_str("payload_type") != Some("run.usage")
    {
        return Ok(Line::Noise);
    }
    let payload = root
        .get("payload")
        .and_then(distinct)
        .ok_or(ParseError::Shape)?;
    match payload.get("allowance") {
        None | Some(Value::Null) => Ok(Line::Bare),
        Some(value) => allowance_of(value).map(Line::Allowance),
    }
}

/// The newest of several candidates. A single candidate never comes here.
fn newest(mut seen: Vec<Allowance>) -> Result<Allowance, ParseError> {
    let latest = seen
        .iter()
        .map(|allowance| allowance.as_of_ms)
        .collect::<Option<Vec<_>>>()
        .and_then(|stamps| stamps.into_iter().max())
        .ok_or(ParseError::Shape)?;
    seen.retain(|allowance| allowance.as_of_ms == Some(latest));
    let first = seen.pop().ok_or(ParseError::Shape)?;
    if seen.iter().all(|other| *other == first) {
        Ok(first)
    } else {
        Err(ParseError::Shape)
    }
}

/// The one reader of an `allowance` object, the vendor's and the cache's alike.
fn allowance_of(value: &Value) -> Result<Allowance, ParseError> {
    let allowance = distinct(value).ok_or(ParseError::Shape)?;
    let window = |key: &str| -> Result<Option<Window>, ParseError> {
        match allowance.get(key) {
            Some(held) if matches!(held, Value::Obj(_)) => {
                let held = distinct(held).ok_or(ParseError::Shape)?;
                Ok(Some(Window {
                    used: percent(held.get("used_percent")),
                    resets_ms: instant(held.get("resets_at_ms")),
                }))
            }
            _ => Ok(None),
        }
    };
    Ok(Allowance {
        tier: allowance
            .get_str("tier")
            .filter(|tier| is_tier(tier))
            .map(str::to_owned),
        window: window("window")?,
        weekly: window("weekly")?,
        as_of_ms: instant(allowance.get("as_of_ms")),
    })
}

/// An integral epoch in milliseconds that is a believable clock.
fn instant(value: Option<&Value>) -> Option<i64> {
    epoch(value).filter(|ms| (1..=MS_MAX).contains(ms))
}

/// A tier name ae may print: `[A-Za-z0-9][A-Za-z0-9 ._&+()/-]{0,39}`.
fn is_tier(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() <= TIER_MAX
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.iter().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b' ' | b'.' | b'_' | b'&' | b'+' | b'(' | b')' | b'/' | b'-'
                )
        })
}

/// What an attempt came back with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// An answer ae could read.
    Done,
    /// The deadline passed and the child was killed.
    TimedOut,
    /// The child wrote past the output ceiling.
    TooLarge,
    /// The child failed in a way that has no class of its own: not started,
    /// not read back, or killed by a signal.
    Failed,
    /// The child exited by itself with this non-zero status.
    Exited(u8),
    /// The child exited cleanly but wrote output that is not UTF-8.
    NotUtf8,
    /// The child exited cleanly and its output held no usable record.
    Unparsed,
}

impl Outcome {
    fn slug(self) -> String {
        match self {
            Self::Done => "ok".to_owned(),
            Self::TimedOut => "timeout".to_owned(),
            Self::TooLarge => "too-large".to_owned(),
            Self::Failed => "failed".to_owned(),
            Self::Exited(code) => format!("exit-{code}"),
            Self::NotUtf8 => "not-utf8".to_owned(),
            Self::Unparsed => "unparsed".to_owned(),
        }
    }

    /// The one spelling [`Outcome::slug`] writes: a status is a plain decimal
    /// in `1..=255`, so no other text reads back as a record ae wrote.
    fn from_slug(slug: &str) -> Option<Self> {
        if let Some(code) = slug.strip_prefix("exit-") {
            let plain = !code.starts_with('0') && code.bytes().all(|byte| byte.is_ascii_digit());
            return plain.then(|| code.parse().ok()).flatten().map(Self::Exited);
        }
        [
            Self::Done,
            Self::TimedOut,
            Self::TooLarge,
            Self::Failed,
            Self::NotUtf8,
            Self::Unparsed,
        ]
        .into_iter()
        .find(|outcome| outcome.slug() == slug)
    }
}

/// The shared cache record: the last paid attempt for one Muse config home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cached {
    /// The canonical Muse config home the attempt was made for.
    pub home: String,
    /// When the attempt was made; the only clock [`Cached::due`] reads.
    pub attempt_at: i64,
    /// What the attempt came back with.
    pub outcome: Outcome,
    /// The allowance of a [`Outcome::Done`] attempt, and only of one.
    pub allowance: Option<Allowance>,
}

impl Cached {
    /// The record, rendered for the cache file.
    #[must_use]
    pub fn render(&self) -> String {
        let mut fields = vec![
            ("v", Value::Num(CACHE_VERSION)),
            ("home", Value::str(self.home.clone())),
            ("attempt_at", Value::Num(self.attempt_at)),
            ("outcome", Value::str(self.outcome.slug())),
        ];
        if let Some(allowance) = &self.allowance {
            fields.push(("allowance", allowance.value()));
        }
        Value::obj(fields).render()
    }

    /// Whether another attempt is due at `now` for `home`: the record is for
    /// another home, or its attempt is [`TTL_SECS`] old or from the future.
    /// Reading a record never moves it.
    #[must_use]
    pub fn due(&self, home: &str, now: i64) -> bool {
        self.home != home || !(0..TTL_SECS).contains(&now.saturating_sub(self.attempt_at))
    }

    /// The held rows, judged at `now`; none for an attempt without an answer.
    #[must_use]
    pub fn rows(&self, now: i64) -> Vec<Row> {
        self.allowance
            .as_ref()
            .map_or_else(Vec::new, |allowance| allowance.rows(now))
    }
}

/// Read a cache record, or `None` for anything ae did not write.
#[must_use]
pub fn parse_cache(bytes: &[u8]) -> Option<Cached> {
    let root = json::parse(std::str::from_utf8(bytes).ok()?).ok()?;
    let root = distinct(&root)?;
    if !matches!(root.get("v"), Some(Value::Num(CACHE_VERSION))) {
        return None;
    }
    let Some(Value::Num(attempt_at)) = root.get("attempt_at") else {
        return None;
    };
    let outcome = Outcome::from_slug(root.get_str("outcome")?)?;
    let allowance = match root.get("allowance") {
        None => None,
        Some(value) => Some(allowance_of(value).ok()?),
    };
    // Only an answer carries an allowance, and an answer always does.
    if (outcome == Outcome::Done) != allowance.is_some() {
        return None;
    }
    Some(Cached {
        home: root.get_str("home")?.to_owned(),
        attempt_at: *attempt_at,
        outcome,
        allowance,
    })
}

#[cfg(test)]
mod tests {
    use super::{Cached, Outcome, parse_cache};

    fn held(outcome: &str) -> String {
        format!(
            r#"{{"v":1,"home":"/synthetic/muse-home","attempt_at":1792000000,"outcome":"{outcome}"}}"#
        )
    }

    #[test]
    fn every_failure_class_survives_a_cache_round_trip() {
        for outcome in [
            Outcome::TimedOut,
            Outcome::TooLarge,
            Outcome::Failed,
            Outcome::Exited(1),
            Outcome::Exited(3),
            Outcome::Exited(255),
            Outcome::NotUtf8,
            Outcome::Unparsed,
        ] {
            let record = Cached {
                home: "/synthetic/muse-home".to_owned(),
                attempt_at: 1_792_000_000,
                outcome,
                allowance: None,
            };
            let reread = parse_cache(record.render().as_bytes()).expect("own record");
            assert_eq!(reread, record, "{outcome:?}");
            assert_eq!(reread.render(), record.render());
        }
    }

    #[test]
    fn only_the_spelling_ae_writes_reads_back_as_an_exit_status() {
        for good in ["exit-1", "exit-9", "exit-10", "exit-255"] {
            assert!(parse_cache(held(good).as_bytes()).is_some(), "{good}");
        }
        for bad in [
            "exit-0",
            "exit-00",
            "exit-01",
            "exit-256",
            "exit-1000",
            "exit-",
            "exit--1",
            "exit-+1",
            "exit-1.0",
            "exit-1x",
            "exit- 1",
            "EXIT-1",
            "exit_1",
            "exit",
            "exit-99999999999999999999",
        ] {
            assert!(parse_cache(held(bad).as_bytes()).is_none(), "{bad}");
        }
    }
}
