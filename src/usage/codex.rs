//! Pure parser for Codex rollout usage records.

use crate::json;
use crate::usage::{Tokens, claude::count};
use std::collections::HashSet;

/// One rollout's last cumulative token event and last turn model.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    /// Every distinct model named by turn contexts, in first-seen order.
    pub models: Vec<String>,
    /// Last cumulative total; cached input is split from billable input.
    pub tokens: Tokens,
    /// A later cumulative total decreased and is therefore approximate.
    pub approximate: bool,
}

/// Parse a bounded Codex rollout tail.
#[must_use]
pub fn parse(bytes: &[u8], starts_at_boundary: bool) -> Parsed {
    parse_with_head(&[], bytes, starts_at_boundary)
}

/// Parse model context from a bounded rollout head and counters/models from its tail.
#[must_use]
pub fn parse_with_head(head: &[u8], tail: &[u8], tail_starts_at_boundary: bool) -> Parsed {
    let mut parsed = parse_tail(tail, tail_starts_at_boundary);
    let mut models = model_names(head, true);
    for model in parsed.models.drain(..) {
        if !models.iter().any(|known| known == &model) {
            models.push(model);
        }
    }
    parsed.models = models;
    parsed
}

fn parse_tail(bytes: &[u8], starts_at_boundary: bool) -> Parsed {
    let text = String::from_utf8_lossy(bytes);
    let mut parsed = Parsed::default();
    let mut models = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 && !starts_at_boundary {
            continue;
        }
        let Ok(value) = json::parse(line) else {
            continue;
        };
        match value.get_str("type") {
            Some("turn_context") => {
                if let Some(model) = value.get("payload").and_then(|p| p.get_str("model"))
                    && super::is_model_id(model)
                    && models.insert(model.to_owned())
                {
                    parsed.models.push(model.to_owned());
                }
            }
            Some("event_msg") => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                if payload.get_str("type") != Some("token_count") {
                    continue;
                }
                let Some(total) = payload
                    .get("info")
                    .and_then(|info| info.get("total_token_usage"))
                else {
                    continue;
                };
                let raw_input = count(total.get("input_tokens"));
                let cached = count(total.get("cached_input_tokens")).min(raw_input);
                let cache_write = count(total.get("cache_write_input_tokens"))
                    .min(raw_input.saturating_sub(cached));
                let next = Tokens {
                    input: raw_input.saturating_sub(cached).saturating_sub(cache_write),
                    cache_write,
                    cache_read: cached,
                    // reasoning_output_tokens is reported as a subset of output_tokens.
                    output: count(total.get("output_tokens")),
                };
                if next.total() < parsed.tokens.total() {
                    parsed.approximate = true;
                }
                parsed.tokens = next;
            }
            _ => {}
        }
    }
    parsed
}

fn model_names(bytes: &[u8], starts_at_boundary: bool) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 && !starts_at_boundary {
            continue;
        }
        let Ok(value) = json::parse(line) else {
            continue;
        };
        if value.get_str("type") != Some("turn_context") {
            continue;
        }
        if let Some(model) = value
            .get("payload")
            .and_then(|payload| payload.get_str("model"))
            && super::is_model_id(model)
            && !found.iter().any(|known| known == model)
        {
            found.push(model.to_owned());
        }
    }
    found
}
