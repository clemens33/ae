//! Codex rollout lines to tool-statistics events (adapter grammar v1).
//!
//! Pure and stateless: one JSONL line in, its [`Event`]s out. Fields read, and
//! only these: root `type`, `timestamp`; `turn_context.payload.{turn_id,
//! model, effort}`; `response_item` payload `type`, `call_id`, `name`,
//! `namespace` and `internal_chat_message_metadata_passthrough.turn_id`;
//! `event_msg.payload.{type, turn_id}`; `item_completed` `item.{type, id,
//! server, tool, status}` and `item.result.isError`. Never `arguments`,
//! `input`, `output`, `result.content`, `cwd` or `compacted.replacement_history`.
//!
//! A line is malformed when it is not JSON, or a record it reads has a field
//! of the wrong type or misses a key: the whole line is then skipped.

use crate::json::{self, Value};
use crate::tool_stats::{self, Event, Ident, Key, Kind, McpOutcome, Parsed};

/// Read one line.
pub fn line(text: &[u8], out: &mut Vec<Event>) -> Parsed {
    let start = out.len();
    let parsed = record(text, out);
    if parsed.is_err() {
        out.truncate(start);
    }
    parsed.unwrap_or(Parsed::Malformed)
}

/// A malformed line.
struct Bad;

fn record(text: &[u8], out: &mut Vec<Event>) -> Result<Parsed, Bad> {
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(Parsed::Ignored);
    }
    let text = std::str::from_utf8(text).map_err(|_| Bad)?;
    let value = json::parse(text).map_err(|_| Bad)?;
    if !matches!(value, Value::Obj(_)) {
        return Err(Bad);
    }
    let kind = match value.get("type") {
        None => return Ok(Parsed::Ignored),
        Some(Value::Str(kind)) => kind.as_str(),
        Some(_) => return Err(Bad),
    };
    let parsed = match kind {
        "turn_context" => context(payload(&value)?, out)?,
        "response_item" => item(payload(&value)?, out)?,
        "event_msg" => event(payload(&value)?, out)?,
        _ => return Ok(Parsed::Ignored),
    };
    if let Some(micros) = value
        .get_str("timestamp")
        .and_then(crate::time::Timestamp::parse_micros)
    {
        out.push(Event::Stamp(micros));
    }
    Ok(parsed)
}

fn payload(value: &Value) -> Result<&Value, Bad> {
    match value.get("payload") {
        Some(payload @ Value::Obj(_)) => Ok(payload),
        _ => Err(Bad),
    }
}

/// An optional string field: absent is `None`, another type is malformed.
fn optional<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, Bad> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(text)) => Ok(Some(text)),
        Some(_) => Err(Bad),
    }
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str, Bad> {
    optional(value, key)?.ok_or(Bad)
}

fn turn(id: Option<&str>) -> Option<Key> {
    id.map(|id| Key::of(b"turn", id.as_bytes()))
}

fn context(payload: &Value, out: &mut Vec<Event>) -> Result<Parsed, Bad> {
    let Some(turn_id) = optional(payload, "turn_id")? else {
        // An older context with no turn id attributes nothing.
        return Ok(Parsed::Ignored);
    };
    let model = optional(payload, "model")?.map_or(Ident::Unresolved(Kind::Model), |model| {
        tool_stats::ident(Kind::Model, model)
    });
    let effort = optional(payload, "effort")?.map_or(Ident::Unresolved(Kind::Effort), |effort| {
        tool_stats::ident(Kind::Effort, effort)
    });
    out.push(Event::Context {
        turn: Key::of(b"turn", turn_id.as_bytes()),
        model,
        effort,
    });
    Ok(Parsed::Kept)
}

fn item(payload: &Value, out: &mut Vec<Event>) -> Result<Parsed, Bad> {
    match optional(payload, "type")? {
        Some("function_call" | "custom_tool_call") => {
            let call = required(payload, "call_id")?;
            let name = required(payload, "name")?;
            let tool = match optional(payload, "namespace")? {
                None => tool_stats::ident(Kind::Tool, name),
                Some(namespace) => match (
                    tool_stats::ident(Kind::Tool, namespace),
                    tool_stats::ident(Kind::Tool, name),
                ) {
                    (Ident::Name(namespace), Ident::Name(name)) => {
                        Ident::Name(format!("{namespace}/{name}"))
                    }
                    _ => Ident::Unresolved(Kind::Tool),
                },
            };
            let metadata = match payload.get("internal_chat_message_metadata_passthrough") {
                None | Some(Value::Null) => None,
                Some(metadata @ Value::Obj(_)) => optional(metadata, "turn_id")?,
                Some(_) => return Err(Bad),
            };
            out.push(Event::Attempt {
                id: Key::of(b"call", call.as_bytes()),
                tool,
                mcp: None,
                model: None,
                turn: turn(metadata),
                message: None,
            });
            Ok(Parsed::Kept)
        }
        Some("function_call_output" | "custom_tool_call_output") => {
            let call = required(payload, "call_id")?;
            out.push(Event::Result {
                id: Key::of(b"call", call.as_bytes()),
                error: false,
                load: None,
            });
            Ok(Parsed::Kept)
        }
        _ => Ok(Parsed::Ignored),
    }
}

fn event(payload: &Value, out: &mut Vec<Event>) -> Result<Parsed, Bad> {
    if optional(payload, "type")? != Some("item_completed") {
        return Ok(Parsed::Ignored);
    }
    let Some(item @ Value::Obj(_)) = payload.get("item") else {
        return Err(Bad);
    };
    if optional(item, "type")? != Some("McpToolCall") {
        return Ok(Parsed::Ignored);
    }
    let id = required(item, "id")?;
    let server = required(item, "server")?;
    let tool = required(item, "tool")?;
    let is_error = match item.get("result") {
        None | Some(Value::Null) => None,
        Some(result @ Value::Obj(_)) => match result.get("isError") {
            None => None,
            Some(Value::Bool(error)) => Some(*error),
            Some(_) => return Err(Bad),
        },
        Some(_) => return Err(Bad),
    };
    let outcome = match (required(item, "status")?, is_error) {
        ("completed", Some(false)) => McpOutcome::Success,
        ("completed", Some(true)) | ("failed", Some(true) | None) => McpOutcome::Error,
        ("completed", None) => McpOutcome::Returned,
        ("failed", Some(false)) => McpOutcome::Conflict,
        _ => return Err(Bad),
    };
    out.push(Event::Mcp {
        id: Key::of(b"mcp", id.as_bytes()),
        server: tool_stats::ident(Kind::Server, server),
        tool: tool_stats::ident(Kind::Tool, tool),
        outcome,
        turn: turn(optional(payload, "turn_id")?),
    });
    Ok(Parsed::Kept)
}
