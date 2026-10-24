//! Pure, bounded-window attribution. Coverage describes recorded evidence, not quota.
use crate::json::{self, Value};
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

const CODES: [&str; 7] = [
    "before_tail",
    "gap",
    "future",
    "budget",
    "unreadable",
    "conflict",
    "unbucketed",
];
#[derive(Default)]
/// Parsed evidence from one bounded source.
pub struct Scan {
    known: [u64; 6],
    reasons: [u8; 6],
    approximate: [bool; 6],
    unbucketed: u64,
    calls: BTreeMap<(String, String), Call>,
}
#[derive(Clone)]
struct Call {
    tokens: [u64; 4],
    model: String,
    time: Option<i64>,
    before: Option<i64>,
    after: Option<i64>,
    approximate: bool,
}
struct Record {
    time: Option<i64>,
    call: Option<Box<((String, String), Call)>>,
    counter: Option<[u64; 4]>,
    last: Option<[u64; 4]>,
}
/// Hourly report ready for either renderer.
pub struct Observation {
    pub(crate) now: i64,
    pub(crate) sessions: Vec<(String, Vec<Value>)>,
    pub(crate) exclusions: Vec<String>,
}
fn oldest(now: i64) -> i64 {
    now.div_euclid(3600) * 3600 - 5 * 3600
}
fn sum(tokens: [u64; 4]) -> u64 {
    tokens.into_iter().fold(0, u64::saturating_add)
}
fn counter(value: Option<&Value>) -> Option<[u64; 4]> {
    let value = value?;
    let required = |key| match value.get(key)? {
        Value::Num(n) => u64::try_from(*n).ok(),
        Value::Raw(n) => n.parse().ok(),
        _ => None,
    };
    Some([
        required("input_tokens")?,
        required("output_tokens")?,
        value
            .get("cached_input_tokens")
            .map_or(Some(0), |_| required("cached_input_tokens"))?,
        value
            .get("cache_write_input_tokens")
            .map_or(Some(0), |_| required("cache_write_input_tokens"))?,
    ])
}
impl Scan {
    pub(crate) fn failed(code: u8, approximate: bool) -> Self {
        Self {
            reasons: [code; 6],
            approximate: [approximate; 6],
            ..Self::default()
        }
    }
    pub(crate) fn failure(&self) -> Option<String> {
        [
            (8, "not read (budget)"),
            (16, "unreadable transcript"),
            (32, "conflict"),
        ]
        .into_iter()
        .find(|(bit, _)| self.reasons.iter().any(|r| r & bit != 0))
        .map(|(_, reason)| reason.to_owned())
    }

    fn taint(&mut self, mut before: Option<i64>, mut after: Option<i64>, code: u8, now: i64) {
        if before.zip(after).is_some_and(|(a, b)| a > b) {
            std::mem::swap(&mut before, &mut after);
        }
        for (i, reason) in self.reasons.iter_mut().enumerate() {
            let start = oldest(now) + i64::try_from(i).unwrap_or(0) * 3600;
            if before.is_none_or(|t| t < start + 3600) && after.is_none_or(|t| t >= start) {
                *reason |= code;
            }
        }
    }
    fn charge(&mut self, call: &Call, now: i64) {
        match call.time {
            None => {
                self.unbucketed = self.unbucketed.saturating_add(sum(call.tokens));
                self.taint(call.before, call.after, 64, now);
            }
            Some(t) if t > now => self.reasons[5] |= 4,
            Some(t) => {
                if let Ok(i) = usize::try_from((t - oldest(now)).div_euclid(3600))
                    && i < 6
                {
                    self.known[i] = self.known[i].saturating_add(sum(call.tokens));
                    self.approximate[i] |= call.approximate;
                }
            }
        }
    }
    fn coverage(&mut self, witness: Option<i64>, whole: bool, now: i64) {
        if !whole {
            for (i, reason) in self.reasons.iter_mut().enumerate() {
                if witness
                    .is_none_or(|t| t > oldest(now) + i64::try_from(i).unwrap_or(0) * 3600 - 600)
                {
                    *reason |= 1;
                }
            }
        }
    }
    pub(crate) fn merge(&mut self, child: Self, parent_ids: &HashSet<String>) {
        for i in 0..6 {
            self.reasons[i] |= child.reasons[i];
        }
        for (key, call) in child.calls {
            if !parent_ids.contains(&key.0) {
                winner(&mut self.calls, key, call);
            }
        }
    }
    pub(crate) fn ids(&self) -> HashSet<String> {
        self.calls.keys().map(|key| key.0.clone()).collect()
    }
    /// Charge the retained Claude winners to their UTC receipt buckets.
    #[must_use]
    pub fn finish(mut self, now: i64) -> Self {
        for call in std::mem::take(&mut self.calls).into_values() {
            self.charge(&call, now);
        }
        self
    }
    pub(crate) fn seat(
        self,
        entry: &crate::meta::RosterEntry,
        retired: bool,
        prior: usize,
        reason: Option<String>,
        now: i64,
    ) -> Value {
        let hours = (0..6)
            .map(|i| {
                let known = self.known[i];
                let complete = self.reasons[i] == 0;
                Value::obj([
                    (
                        "start",
                        Value::Num(oldest(now) + i64::try_from(i).unwrap_or(0) * 3600),
                    ),
                    ("open", Value::Bool(i == 5)),
                    (
                        "tokens",
                        if complete {
                            super::json_u64(known)
                        } else {
                            Value::Null
                        },
                    ),
                    ("known_tokens", super::json_u64(known)),
                    (
                        "coverage",
                        Value::str(if complete {
                            "complete"
                        } else if known > 0 {
                            "partial"
                        } else {
                            "unknown"
                        }),
                    ),
                    ("approximate", Value::Bool(self.approximate[i])),
                    (
                        "reasons",
                        Value::Arr(
                            CODES
                                .iter()
                                .enumerate()
                                .filter(|(bit, _)| self.reasons[i] & (1 << bit) != 0)
                                .map(|(_, code)| Value::str(*code))
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect();
        Value::obj([
            ("seat", Value::str(&entry.name)),
            ("slot", Value::str(&entry.slot)),
            ("tool", Value::str(entry.binary.as_deref().unwrap_or("?"))),
            ("retired", Value::Bool(retired)),
            ("coverage_reason", reason.map_or(Value::Null, Value::str)),
            ("unbucketed_tokens", super::json_u64(self.unbucketed)),
            ("predecessors_not_read", super::json_usize(prior)),
            ("hours", Value::Arr(hours)),
        ])
    }
}
fn winner(calls: &mut BTreeMap<(String, String), Call>, key: (String, String), mut call: Call) {
    if let Some(previous) = calls.get(&key) {
        let equal = sum(call.tokens) == sum(previous.tokens);
        if sum(call.tokens) < sum(previous.tokens) {
            return;
        }
        call.approximate |= equal
            && (call.tokens != previous.tokens
                || call.model != previous.model
                || previous.approximate);
        if equal && previous.time.is_some() && (call.time.is_none() || previous.time <= call.time) {
            call = Call {
                approximate: call.approximate,
                ..previous.clone()
            };
        }
    }
    calls.insert(key, call);
}
/// Parse a bounded Claude or Codex tail; counters remain in file order.
#[must_use]
pub fn parse(bytes: &[u8], boundary: bool, whole: bool, now: i64, codex: bool) -> Scan {
    let records = records(bytes, boundary, codex);
    let witness = records
        .iter()
        .find(|r| r.time.is_some() && (!codex || r.counter.is_some()))
        .and_then(|r| r.time);
    let zero = whole
        && (!codex
            || records
                .first()
                .is_some_and(|r| r.counter.is_some() && r.counter == r.last));
    let mut scan = Scan {
        approximate: [codex; 6],
        ..Scan::default()
    };
    scan.coverage(witness, zero, now);
    let mut next = None;
    let mut after = vec![None; records.len()];
    for (i, record) in records.iter().enumerate().rev() {
        after[i] = next;
        if record.time.is_some() {
            next = record.time;
        }
    }
    let mut before = None;
    let mut baseline = zero.then_some([0; 4]);
    for (i, record) in records.into_iter().enumerate() {
        if codex {
            if let Some(total) = record.counter {
                if let Some(previous) = baseline {
                    if total.iter().zip(previous).any(|(a, b)| *a < b)
                        || record.time.zip(before).is_some_and(|(a, b)| a < b)
                    {
                        scan.taint(before, record.time.or(after[i]), 2, now);
                    } else {
                        scan.charge(
                            &Call {
                                tokens: [total[0] - previous[0], total[1] - previous[1], 0, 0],
                                model: String::new(),
                                time: record.time,
                                before,
                                after: after[i],
                                approximate: true,
                            },
                            now,
                        );
                    }
                }
                baseline = Some(total);
            } else {
                scan.taint(before, after[i], 2, now);
                baseline = None;
            }
        } else if let Some(call) = record.call {
            let (key, mut call) = *call;
            call.before = before;
            call.after = after[i];
            winner(&mut scan.calls, key, call);
        } else if record.last.is_some() {
            scan.taint(before, after[i], 2, now);
        }
        if record.time.is_some() {
            before = record.time;
        }
    }
    scan
}
/// Render six UTC buckets, preserving unknown and approximate evidence.
#[must_use]
pub fn render(observation: &Observation, json: bool) -> String {
    if json {
        return Value::obj([
            ("as_of", Value::Num(observation.now)),
            ("open_hour", Value::Num(oldest(observation.now) + 5 * 3600)),
            (
                "sessions",
                Value::Arr(
                    observation
                        .sessions
                        .iter()
                        .map(|(name, seats)| {
                            Value::obj([
                                ("name", Value::str(name)),
                                ("seats", Value::Arr(seats.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "exclusions",
                Value::Arr(observation.exclusions.iter().map(Value::str).collect()),
            ),
        ])
        .render()
            + "\n";
    }
    let mut out = format!(
        "UTC {}–{}; ~ approximate, +? partial, ? unknown\n",
        crate::time::Timestamp::from_epoch(oldest(observation.now)),
        crate::time::Timestamp::from_epoch(observation.now)
    );
    let _ = writeln!(
        out,
        "{:<32} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "seat", "-5", "-4", "-3", "-2", "-1", "now"
    );
    for (session, seats) in &observation.sessions {
        for seat in seats {
            let label = format!("{session}:{}", seat.get_str("seat").unwrap_or("?"));
            let Some(Value::Arr(hours)) = seat.get("hours") else {
                continue;
            };
            let cells = hours
                .iter()
                .map(|hour| {
                    let n = super::claude::count(hour.get("known_tokens"));
                    let complete = hour.get_str("coverage") == Some("complete");
                    if n == 0 && !complete {
                        return "?".to_owned();
                    }
                    let base = if n < 1_000_000_000 {
                        super::human(n)
                    } else {
                        let digits = n.to_string();
                        format!("{}.{}e{}", &digits[..1], &digits[1..2], digits.len() - 1)
                    };
                    format!(
                        "{base}{}{}",
                        if hour.get("approximate") == Some(&Value::Bool(true)) {
                            "~"
                        } else {
                            ""
                        },
                        if complete { "" } else { "+?" }
                    )
                })
                .map(|cell| format!("{cell:>9}"))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = writeln!(out, "{label:<32} {cells}");
            let unbucketed = super::claude::count(seat.get("unbucketed_tokens"));
            if unbucketed > 0 {
                let _ = writeln!(out, "unbucketed {label}: {unbucketed}");
            }
        }
    }
    for exclusion in &observation.exclusions {
        out.push_str(exclusion);
        out.push('\n');
    }
    out
}

fn records(bytes: &[u8], boundary: bool, codex: bool) -> Vec<Record> {
    let mut records: Vec<Record> = Vec::new();
    for (i, line) in bytes.split_inclusive(|b| *b == b'\n').enumerate() {
        if (i == 0 && !boundary)
            || line.last() != Some(&b'\n')
            || line.iter().all(u8::is_ascii_whitespace)
        {
            continue;
        }
        let value = std::str::from_utf8(line)
            .ok()
            .and_then(|line| json::parse(line).ok());
        let time = value
            .as_ref()
            .and_then(|v| v.get_str("timestamp"))
            .and_then(crate::time::Timestamp::parse_micros)
            .map(|t| t.div_euclid(1_000_000));
        let mut record = Record {
            time,
            call: None,
            counter: None,
            last: None,
        };
        if codex {
            let Some(payload) = value
                .as_ref()
                .filter(|v| v.get_str("type") == Some("event_msg"))
                .and_then(|v| v.get("payload"))
                .filter(|v| v.get_str("type") == Some("token_count"))
            else {
                continue;
            };
            record.counter = counter(payload.get("info").and_then(|v| v.get("total_token_usage")));
            record.last = counter(payload.get("info").and_then(|v| v.get("last_token_usage")));
        } else if let Some(value) = value {
            if !matches!(value.get_str("type"), Some("user" | "assistant")) {
                continue;
            }
            if value.get_str("type") == Some("assistant")
                && let Some(message) = value.get("message")
            {
                let Some(model) = message
                    .get_str("model")
                    .filter(|m| *m != "<synthetic>" && super::is_model_id(m))
                else {
                    continue;
                };
                if let Some(usage) = message.get("usage") {
                    if let Some((id, request)) = message
                        .get_str("id")
                        .filter(|s| !s.is_empty())
                        .zip(value.get_str("requestId").filter(|s| !s.is_empty()))
                    {
                        record.call = Some(Box::new((
                            (id.to_owned(), request.to_owned()),
                            Call {
                                tokens: [
                                    "input_tokens",
                                    "cache_creation_input_tokens",
                                    "cache_read_input_tokens",
                                    "output_tokens",
                                ]
                                .map(|k| super::claude::count(usage.get(k))),
                                model: model.to_owned(),
                                time,
                                before: None,
                                after: None,
                                approximate: false,
                            },
                        )));
                    } else {
                        record.last = Some([0; 4]);
                    }
                }
            }
        } else {
            record.last = Some([0; 4]);
        }
        if !codex && record.time.is_none() && record.call.is_none() && record.last.is_none() {
            continue;
        }
        if record.time.is_none()
            && record.call.is_none()
            && record.counter.is_none()
            && records
                .last()
                .is_some_and(|r| r.time.is_none() && r.call.is_none() && r.counter.is_none())
        {
            continue;
        }
        records.push(record);
    }
    records
}
