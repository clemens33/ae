//! Pure parse and reduction of Claude Code assistant transcript JSONL.

use crate::json::{self, Value};
use crate::usage::Tokens;
use std::collections::{HashMap, HashSet};

/// Parsed records from one parent or sidechain transcript.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    entries: Vec<Entry>,
}

/// Incremental transcript parser used by the bounded file reader.
#[derive(Debug, Default)]
pub struct Parser {
    parsed: Parsed,
    keyed: HashMap<(String, Option<String>), usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    message_id: Option<String>,
    request_id: Option<String>,
    model: String,
    tokens: Tokens,
}

/// One model's reduced usage within a seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelUsage {
    pub model: String,
    pub tokens: Tokens,
}

/// Parse one transcript and keep the greater streaming snapshot per message/request pair.
#[must_use]
pub fn parse(bytes: &[u8]) -> Parsed {
    let mut parser = Parser::default();
    for line in bytes.split(|byte| *byte == b'\n') {
        parser.push(line);
    }
    parser.finish()
}

impl Parser {
    /// Parse one complete JSONL record, ignoring malformed or irrelevant input.
    pub fn push(&mut self, line: &[u8]) {
        let text = String::from_utf8_lossy(line);
        let Ok(value) = json::parse(&text) else {
            return;
        };
        let Some(entry) = entry(&value) else {
            return;
        };
        let key = entry
            .message_id
            .as_ref()
            .map(|message_id| (message_id.clone(), entry.request_id.clone()));
        let duplicate = key.as_ref().and_then(|key| self.keyed.get(key).copied());
        if let Some(at) = duplicate {
            if entry.tokens.total() > self.parsed.entries[at].tokens.total() {
                self.parsed.entries[at] = entry;
            }
        } else {
            if let Some(key) = key {
                self.keyed.insert(key, self.parsed.entries.len());
            }
            self.parsed.entries.push(entry);
        }
    }

    /// Finish the stream and return its deduplicated entries.
    #[must_use]
    pub fn finish(self) -> Parsed {
        self.parsed
    }
}

/// Reduce a parent transcript and its sidechains, skipping parent-message replays.
#[must_use]
pub fn reduce(parent: &Parsed, sidechains: &[Parsed]) -> Vec<ModelUsage> {
    let parent_ids = parent
        .entries
        .iter()
        .filter_map(|entry| entry.message_id.as_deref())
        .collect::<HashSet<_>>();
    let mut combined = parent.entries.clone();
    let mut keyed = combined
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            entry
                .message_id
                .as_ref()
                .map(|message_id| ((message_id.clone(), entry.request_id.clone()), index))
        })
        .collect::<HashMap<_, _>>();
    for entry in sidechains.iter().flat_map(|parsed| parsed.entries.iter()) {
        if entry
            .message_id
            .as_deref()
            .is_some_and(|id| parent_ids.contains(&id))
        {
            continue;
        }
        let key = entry
            .message_id
            .as_ref()
            .map(|message_id| (message_id.clone(), entry.request_id.clone()));
        let duplicate = key.as_ref().and_then(|key| keyed.get(key).copied());
        if let Some(at) = duplicate {
            if entry.tokens.total() > combined[at].tokens.total() {
                combined[at] = entry.clone();
            }
        } else {
            if let Some(key) = key {
                keyed.insert(key, combined.len());
            }
            combined.push(entry.clone());
        }
    }
    let mut models: Vec<ModelUsage> = Vec::new();
    let mut model_indices: HashMap<String, usize> = HashMap::new();
    for entry in combined {
        if let Some(index) = model_indices.get(&entry.model).copied() {
            models[index].tokens = models[index].tokens.saturating_add(entry.tokens);
        } else {
            model_indices.insert(entry.model.clone(), models.len());
            models.push(ModelUsage {
                model: entry.model,
                tokens: entry.tokens,
            });
        }
    }
    models
}

fn entry(value: &Value) -> Option<Entry> {
    if value.get_str("type") != Some("assistant") {
        return None;
    }
    let message = value.get("message")?;
    let model = message.get_str("model")?;
    if model == "<synthetic>" || !super::is_model_id(model) {
        return None;
    }
    let usage = message.get("usage")?;
    Some(Entry {
        message_id: message
            .get_str("id")
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        request_id: value
            .get_str("requestId")
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        model: model.to_owned(),
        tokens: Tokens {
            input: count(usage.get("input_tokens")),
            cache_write: count(usage.get("cache_creation_input_tokens")),
            cache_read: count(usage.get("cache_read_input_tokens")),
            output: count(usage.get("output_tokens")),
        },
    })
}

pub(crate) fn count(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Num(number)) => u64::try_from(*number).unwrap_or(0),
        Some(Value::Raw(raw)) => raw.parse().unwrap_or(0),
        _ => 0,
    }
}
