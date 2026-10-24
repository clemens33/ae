//! Versioned quota evidence: pure grammar and fleet folding; filesystem door in `store`.
use crate::json::{self, Value};
use std::collections::BTreeMap;

mod store;
pub use store::{append, read_fleet};

pub const ALIGN_SECS: i64 = 900;
const LINE_CAP: usize = 65_536;
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub reason: &'static str,
}
const BAD: Error = Error {
    reason: "invalid quota history row",
};
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Scope {
    pub tool: String,
    pub source: String,
    pub account: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PolicyFacts {
    pub version: u8,
    pub manual_resets: Option<u8>,
    pub credits: String,
    pub spend_cap: Option<bool>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub producer: String,
    pub incarnation: String,
    pub collected_at: i64,
    pub scope: Scope,
    pub bucket: String,
    pub qualifier: Option<String>,
    pub window_minutes: Option<u32>,
    pub resets_at: i64,
    pub observed_at: i64,
    pub raw: String,
    pub effective: String,
    pub derivation: String,
    pub policy: PolicyFacts,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Seat {
    pub slot: String,
    pub agent: String,
    pub launch: Option<String>,
    pub observed: String,
    pub declared: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Workload {
    pub producer: String,
    pub incarnation: String,
    pub collected_at: i64,
    pub scope: Scope,
    pub observed_at: Option<i64>,
    pub seats: Vec<Seat>,
    pub unscoped: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Sample(Sample),
    Workload(Workload),
}
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub sample: usize,
    pub reason: String,
    pub correction: bool,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skipped {
    pub damaged: usize,
    pub oversize: usize,
    pub foreign: usize,
    pub link: usize,
    pub nonregular: usize,
    pub unreadable: usize,
    pub partial: usize,
    pub truncated: usize,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub samples: Vec<Sample>,
    pub workload: Vec<Workload>,
    pub aligned: Vec<Vec<usize>>,
    pub segments: Vec<Segment>,
    pub skipped: Skipped,
}

impl Row {
    fn identity(&self) -> (&str, &str, i64, &Scope) {
        match self {
            Self::Sample(s) => (&s.producer, &s.incarnation, s.collected_at, &s.scope),
            Self::Workload(w) => (&w.producer, &w.incarnation, w.collected_at, &w.scope),
        }
    }
}
fn optional<T>(value: Option<T>, f: impl FnOnce(T) -> Value) -> Value {
    value.map_or(Value::Null, f)
}
fn scope_value(s: &Scope) -> Value {
    Value::obj([
        ("tool", Value::str(&s.tool)),
        ("source", Value::str(&s.source)),
        ("account", optional(s.account.as_deref(), Value::str)),
    ])
}
fn policy_value(p: &PolicyFacts) -> Value {
    Value::obj([
        ("version", Value::Num(i64::from(p.version))),
        (
            "manual_resets",
            optional(p.manual_resets, |n| Value::Num(i64::from(n))),
        ),
        ("credits", Value::str(&p.credits)),
        ("spend_cap", optional(p.spend_cap, Value::Bool)),
    ])
}
/// Render one row, without a newline. Validation belongs to [`parse`] and [`append`].
#[must_use]
pub fn render(row: &Row) -> String {
    let (producer, incarnation, collected_at, scope) = row.identity();
    let mut fields = vec![
        ("v", Value::Num(1)),
        (
            "kind",
            Value::str(match row {
                Row::Sample(_) => "sample",
                Row::Workload(_) => "workload",
            }),
        ),
        ("producer", Value::str(producer)),
        ("incarnation", Value::str(incarnation)),
        ("collected_at", Value::Num(collected_at)),
        ("scope", scope_value(scope)),
    ];
    match row {
        Row::Sample(s) => fields.extend([
            ("bucket", Value::str(&s.bucket)),
            ("qualifier", optional(s.qualifier.as_deref(), Value::str)),
            (
                "window_minutes",
                optional(s.window_minutes, |n| Value::Num(i64::from(n))),
            ),
            ("resets_at", Value::Num(s.resets_at)),
            ("observed_at", Value::Num(s.observed_at)),
            ("raw", Value::str(&s.raw)),
            ("effective", Value::str(&s.effective)),
            ("derivation", Value::str(&s.derivation)),
            ("policy", policy_value(&s.policy)),
        ]),
        Row::Workload(w) => fields.extend([
            ("observed_at", optional(w.observed_at, Value::Num)),
            (
                "unscoped",
                Value::Num(i64::try_from(w.unscoped).unwrap_or(i64::MAX)),
            ),
            (
                "seats",
                Value::Arr(
                    w.seats
                        .iter()
                        .map(|s| {
                            Value::obj([
                                ("slot", Value::str(&s.slot)),
                                ("agent", Value::str(&s.agent)),
                                ("launch", optional(s.launch.as_deref(), Value::str)),
                                ("observed", Value::str(&s.observed)),
                                ("declared", optional(s.declared.as_deref(), Value::str)),
                                ("model", optional(s.model.as_deref(), Value::str)),
                                ("effort", optional(s.effort.as_deref(), Value::str)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
    }
    Value::obj(fields).render()
}
fn check(yes: bool) -> Result<(), Error> {
    if yes { Ok(()) } else { Err(BAD) }
}
fn object<'a>(v: &'a Value, known: &str) -> Result<&'a Value, Error> {
    let Value::Obj(fields) = v else {
        return Err(BAD);
    };
    let mut seen = std::collections::BTreeSet::new();
    for (key, _) in fields {
        if known.split_whitespace().any(|s| s == key) {
            check(seen.insert(key))?;
        }
    }
    Ok(v)
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value, Error> {
    v.get(key).ok_or(BAD)
}
fn text(v: &Value, cap: usize) -> Result<String, Error> {
    let s = v.as_str().ok_or(BAD)?;
    check(!s.is_empty() && s.len() <= cap && !s.chars().any(char::is_control))?;
    Ok(s.to_owned())
}
fn string(v: &Value, key: &str) -> Result<String, Error> {
    text(field(v, key)?, 256)
}
fn maybe<T>(
    v: &Value,
    key: &str,
    f: impl FnOnce(&Value) -> Result<T, Error>,
) -> Result<Option<T>, Error> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => f(value).map(Some),
    }
}
fn number(v: &Value) -> Result<i64, Error> {
    if let Value::Num(n) = v {
        Ok(*n)
    } else {
        Err(BAD)
    }
}
fn epoch(v: &Value) -> Result<i64, Error> {
    let n = number(v)?;
    check(n > 0)?;
    Ok(n)
}
fn uuid(s: &str) -> bool {
    crate::events::is_session_id(s)
}
fn word(s: &str, words: &str) -> Result<(), Error> {
    check(words.split_whitespace().any(|w| w == s))
}
fn scope_parse(v: &Value) -> Result<Scope, Error> {
    object(v, "tool source account")?;
    let source = text(field(v, "source")?, 4096)?;
    let account = maybe(v, "account", |v| text(v, 4096))?;
    check(
        std::path::Path::new(&source).is_absolute()
            && account
                .as_ref()
                .is_none_or(|p| std::path::Path::new(p).is_absolute()),
    )?;
    Ok(Scope {
        tool: string(v, "tool")?,
        source,
        account,
    })
}
fn percent(s: &str) -> Result<f64, Error> {
    let n = s.parse::<f64>().map_err(|_| BAD)?;
    check(s.len() <= 32 && n.is_finite() && n >= 0.0)?;
    Ok(n)
}
fn policy_parse(v: &Value) -> Result<PolicyFacts, Error> {
    let p = object(v, "version manual_resets credits spend_cap")?;
    check(number(field(p, "version")?)? == 1)?;
    let credits = string(p, "credits")?;
    word(
        &credits,
        "unreported unlimited available available-unknown exhausted",
    )?;
    Ok(PolicyFacts {
        version: 1,
        manual_resets: maybe(p, "manual_resets", |v| {
            u8::try_from(number(v)?).map_err(|_| BAD)
        })?,
        credits,
        spend_cap: maybe(p, "spend_cap", |v| {
            if let Value::Bool(b) = v {
                Ok(*b)
            } else {
                Err(BAD)
            }
        })?,
    })
}
/// Parse a bounded version-1 line; unknown fields are inert.
/// # Errors
/// Invalid JSON, duplicate known fields, types, versions or bounds.
pub fn parse(line: &str) -> Result<Row, Error> {
    check(line.len() <= LINE_CAP && !line.contains(['\n', '\r']))?;
    let v = json::parse(line).map_err(|_| BAD)?;
    object(
        &v,
        "v kind producer incarnation collected_at scope bucket qualifier window_minutes resets_at observed_at raw effective derivation policy seats unscoped",
    )?;
    check(number(field(&v, "v")?)? == 1)?;
    let producer = string(&v, "producer")?;
    let incarnation = string(&v, "incarnation")?;
    check(uuid(&incarnation))?;
    let collected_at = epoch(field(&v, "collected_at")?)?;
    let scope = scope_parse(field(&v, "scope")?)?;
    match v.get_str("kind") {
        Some("sample") => {
            let policy = policy_parse(field(&v, "policy")?)?;
            let raw = string(&v, "raw")?;
            percent(&raw)?;
            let effective = string(&v, "effective")?;
            percent(&effective)?;
            check(effective.split_once('.').is_some_and(|(a, b)| {
                !a.is_empty()
                    && a.bytes().all(|c| c.is_ascii_digit())
                    && b.len() == 6
                    && b.bytes().all(|c| c.is_ascii_digit())
            }))?;
            let derivation = string(&v, "derivation")?;
            word(&derivation, "raw resets unlimited spend-cap")?;
            Ok(Row::Sample(Sample {
                producer,
                incarnation,
                collected_at,
                scope,
                bucket: string(&v, "bucket")?,
                qualifier: maybe(&v, "qualifier", |v| text(v, 256))?,
                window_minutes: maybe(&v, "window_minutes", |v| {
                    u32::try_from(epoch(v)?).map_err(|_| BAD)
                })?,
                resets_at: epoch(field(&v, "resets_at")?)?,
                observed_at: epoch(field(&v, "observed_at")?)?,
                raw,
                effective,
                derivation,
                policy,
            }))
        }
        Some("workload") => {
            let Value::Arr(values) = field(&v, "seats")? else {
                return Err(BAD);
            };
            let unscoped = usize::try_from(number(field(&v, "unscoped")?)?).map_err(|_| BAD)?;
            check(values.len() <= 128 && unscoped <= 128)?;
            let seats = values
                .iter()
                .map(|v| {
                    object(v, "slot agent launch observed declared model effort")?;
                    let launch = maybe(v, "launch", |v| text(v, 256))?;
                    check(launch.as_deref().is_none_or(uuid))?;
                    let observed = string(v, "observed")?;
                    word(&observed, "busy idle unknown")?;
                    let declared = maybe(v, "declared", |v| text(v, 256))?;
                    if let Some(s) = &declared {
                        word(s, "working waiting-user waiting-agent blocked done")?;
                    }
                    Ok(Seat {
                        slot: string(v, "slot")?,
                        agent: string(v, "agent")?,
                        launch,
                        observed,
                        declared,
                        model: maybe(v, "model", |v| text(v, 256))?,
                        effort: maybe(v, "effort", |v| text(v, 256))?,
                    })
                })
                .collect::<Result<_, Error>>()?;
            Ok(Row::Workload(Workload {
                producer,
                incarnation,
                collected_at,
                scope,
                observed_at: maybe(&v, "observed_at", epoch)?,
                seats,
                unscoped,
            }))
        }
        _ => Err(BAD),
    }
}

type Series<'a> = (
    &'a Scope,
    &'a str,
    Option<&'a str>,
    Option<u32>,
    i64,
    &'a PolicyFacts,
    i64,
);
fn series(s: &Sample) -> Series<'_> {
    (
        &s.scope,
        &s.bucket,
        s.qualifier.as_deref(),
        s.window_minutes,
        s.resets_at,
        &s.policy,
        s.observed_at,
    )
}
fn track(s: &Sample) -> (&str, &Scope, &str, Option<&str>, Option<u32>) {
    (
        &s.incarnation,
        &s.scope,
        &s.bucket,
        s.qualifier.as_deref(),
        s.window_minutes,
    )
}
fn numeric(s: &str) -> f64 {
    s.parse().unwrap_or_default()
}
/// Fold validated rows without reading the world. Also exercised by the fuzz target.
#[doc(hidden)]
#[must_use]
pub fn fold(rows: Vec<Row>) -> Report {
    let mut report = Report::default();
    for row in rows.into_iter().take(65_536) {
        match row {
            Row::Sample(s) => report.samples.push(s),
            Row::Workload(w) => report.workload.push(w),
        }
    }
    report.samples.sort_by(|a, b| {
        (track(a), a.observed_at, a.collected_at).cmp(&(track(b), b.observed_at, b.collected_at))
    });
    let mut changes = Vec::new();
    for pair in report.samples.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if track(a) != track(b) {
            continue;
        }
        let drop = numeric(&a.raw) - numeric(&b.raw);
        let reason = if a.resets_at != b.resets_at {
            "window"
        } else if a.policy != b.policy {
            "policy"
        } else if drop > 0.0 {
            "drop"
        } else {
            continue;
        };
        // One billionth of a percentage point absorbs decimal-to-f64 subtraction
        // roundoff (4.4 - 2.4 > 2.0), below the six-place effective precision.
        changes.push((b.clone(), reason, reason == "drop" && drop <= 2.0 + 1e-9));
    }
    report.samples.sort_by(|a, b| {
        series(a)
            .cmp(&series(b))
            .then_with(|| numeric(&b.raw).total_cmp(&numeric(&a.raw)))
            .then_with(|| numeric(&b.effective).total_cmp(&numeric(&a.effective)))
            .then_with(|| {
                (a.collected_at, &a.producer, &a.incarnation).cmp(&(
                    b.collected_at,
                    &b.producer,
                    &b.incarnation,
                ))
            })
    });
    report.samples.dedup_by(|a, b| series(a) == series(b));
    let indices: BTreeMap<_, _> = report
        .samples
        .iter()
        .enumerate()
        .map(|(i, s)| (series(s), i))
        .collect();
    let mut segments: Vec<_> = changes
        .iter()
        .filter_map(|(s, why, c)| indices.get(&series(s)).map(|i| (*i, *why, *c)))
        .collect();
    segments.sort_unstable();
    segments.dedup();
    report.segments = segments
        .into_iter()
        .map(|(sample, reason, correction)| Segment {
            sample,
            reason: reason.to_owned(),
            correction,
        })
        .collect();
    report.workload.sort_by(|a, b| {
        (
            &a.scope,
            &a.incarnation,
            a.observed_at,
            a.collected_at,
            &a.producer,
        )
            .cmp(&(
                &b.scope,
                &b.incarnation,
                b.observed_at,
                b.collected_at,
                &b.producer,
            ))
    });
    report.aligned = align(&report);
    report
}

fn align(report: &Report) -> Vec<Vec<usize>> {
    report
        .samples
        .iter()
        .map(|s| {
            let mut newest = BTreeMap::new();
            for (i, w) in report
                .workload
                .iter()
                .enumerate()
                .filter(|(_, w)| w.scope == s.scope)
            {
                if let Some(at) = w.observed_at.filter(|at| {
                    *at <= s.observed_at
                        && s.observed_at.saturating_sub(*at) <= ALIGN_SECS
                        && *at <= w.collected_at
                        && w.collected_at.saturating_sub(*at) <= ALIGN_SECS
                }) {
                    let candidate = (at, w.collected_at, i);
                    let held = newest.entry(&w.incarnation).or_insert(candidate);
                    if candidate > *held {
                        *held = candidate;
                    }
                }
            }
            newest.into_values().map(|(_, _, i)| i).collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Row, fold, parse};

    #[test]
    fn decimal_drops_at_two_points_are_corrections() {
        let Row::Sample(mut a) = parse(r#"{"v":1,"kind":"sample","producer":"s","incarnation":"11111111-1111-4111-8111-111111111111","collected_at":2000029950,"scope":{"source":"/h/.claude.json","tool":"claude","account":"/h"},"bucket":"weekly_all","window_minutes":10080,"resets_at":2000289200,"observed_at":2000029940,"raw":"41","effective":"41.000000","derivation":"raw","policy":{"credits":"unreported","version":1,"manual_resets":null}}"#).unwrap() else {
            panic!("sample seed");
        };
        for (before, after, correction) in [
            ("4.4", "2.4", true),
            ("4.03", "2.03", true),
            ("4.4", "2.39", false),
        ] {
            a.raw = before.to_owned();
            let mut b = a.clone();
            b.raw = after.to_owned();
            b.observed_at += 1;
            let report = fold(vec![Row::Sample(a.clone()), Row::Sample(b)]);
            assert_eq!(report.segments.len(), 1);
            assert_eq!(report.segments[0].reason, "drop");
            assert_eq!(
                report.segments[0].correction, correction,
                "{before} -> {after}"
            );
        }
    }
}
