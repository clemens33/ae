//! Claude Code transcript lines to tool-statistics events (adapter grammar v1).
//!
//! Pure and stateless: one JSONL line in, its [`Event`]s out. Fields read,
//! and only these: root `type`, `timestamp`, `isMeta`, `sourceToolUseID`,
//! `toolUseResult.success`; `message.model`, `message.id`; content blocks'
//! `type`, `id`, `name`, `tool_use_id`, `is_error`; `input.skill` of a
//! `Skill` call; `input.file_path` of a `Read` call (Q2 b, digested, dropped);
//! the first text line of an `isMeta` record linked by `sourceToolUseID`, only
//! when it starts with [`crate::tool_stats::BASE_PREFIX`] (Q1 b, classified,
//! digested, dropped). Never `input.args` or any other `input` key, `content`,
//! `text` or `thinking` beyond that one line, other `toolUseResult` keys,
//! `cwd` or `attribution*`.
//!
//! A line is malformed when it is not JSON, or a record it reads has a field
//! of the wrong type or misses a key: the whole line is then skipped.

use crate::json::{self, Value};
use crate::tool_stats::{
    self, Adapter, BASE_PREFIX, Dir, Event, Ident, Key, Kind, Load, Parsed, READ_PATH_MAX,
    Resource, ScopeClass,
};
use std::path::PathBuf;

/// The seat roots a skill directory is classified against: every recorded
/// config home and work dir of the conversation's memberships. In memory only.
#[derive(Debug, Clone, Copy)]
pub struct Roots<'a> {
    pub config_homes: &'a [PathBuf],
    pub work_dirs: &'a [PathBuf],
}

/// The model word a synthetic (harness-written) assistant record carries.
const SYNTHETIC: &str = "<synthetic>";

/// Read one line.
pub fn line(text: &[u8], roots: &Roots<'_>, out: &mut Vec<Event>) -> Parsed {
    let start = out.len();
    let parsed = record(text, roots, out);
    if parsed.is_err() {
        out.truncate(start);
    }
    parsed.unwrap_or(Parsed::Malformed)
}

/// A malformed line.
struct Bad;

fn record(text: &[u8], roots: &Roots<'_>, out: &mut Vec<Event>) -> Result<Parsed, Bad> {
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
    match kind {
        "assistant" => {
            stamp(&value, out);
            assistant(&value, roots, out)
        }
        "user" => {
            stamp(&value, out);
            user(&value, roots, out)
        }
        _ => Ok(Parsed::Ignored),
    }
}

/// The record's native timestamp, when it has a readable one.
fn stamp(value: &Value, out: &mut Vec<Event>) {
    if let Some(micros) = value
        .get_str("timestamp")
        .and_then(crate::time::Timestamp::parse_micros)
    {
        out.push(Event::Stamp(micros));
    }
}

fn assistant(value: &Value, roots: &Roots<'_>, out: &mut Vec<Event>) -> Result<Parsed, Bad> {
    match value.get("isMeta") {
        // A harness-injected record is never a model's call.
        Some(Value::Bool(true)) => return Ok(Parsed::Ignored),
        None | Some(Value::Bool(false)) => {}
        Some(_) => return Err(Bad),
    }
    let message = value.get("message").ok_or(Bad)?;
    let model = match message.get("model") {
        None => None,
        Some(Value::Str(model)) if model == SYNTHETIC => None,
        Some(Value::Str(model)) => Some(tool_stats::ident(Kind::Model, model)),
        Some(_) => return Err(Bad),
    };
    let message_key = match message.get("id") {
        None => None,
        Some(Value::Str(id)) => Some(Key::of(b"message", id.as_bytes())),
        Some(_) => return Err(Bad),
    };
    let blocks = match message.get("content") {
        Some(Value::Arr(blocks)) => blocks,
        Some(Value::Str(_)) => return Ok(Parsed::Kept),
        _ => return Err(Bad),
    };
    for block in blocks {
        if block.get_str("type") != Some("tool_use") {
            continue;
        }
        let id = block.get_str("id").ok_or(Bad)?;
        let name = block.get_str("name").ok_or(Bad)?;
        let key = Key::of(b"call", id.as_bytes());
        out.push(Event::Attempt {
            id: key,
            tool: tool_stats::ident(Kind::Tool, name),
            mcp: mcp(name),
            model: model.clone(),
            turn: None,
            message: message_key,
        });
        let input = block.get("input");
        match name {
            "Skill" => {
                let skill = input
                    .and_then(|input| input.get_str("skill"))
                    .map_or(Ident::Unresolved(Kind::Skill), |skill| {
                        tool_stats::ident(Kind::Skill, skill)
                    });
                out.push(Event::Skill { id: key, skill });
            }
            "Read" => {
                if let Some(path) = input.and_then(|input| input.get_str("file_path")) {
                    out.extend(read(key, path, roots));
                }
            }
            _ => {}
        }
    }
    Ok(Parsed::Kept)
}

fn user(value: &Value, roots: &Roots<'_>, out: &mut Vec<Event>) -> Result<Parsed, Bad> {
    match value.get("isMeta") {
        Some(Value::Bool(true)) => return Ok(base(value, roots, out)),
        None | Some(Value::Bool(false)) => {}
        Some(_) => return Err(Bad),
    }
    let message = value.get("message").ok_or(Bad)?;
    let blocks = match message.get("content") {
        Some(Value::Arr(blocks)) => blocks,
        Some(Value::Str(_)) => return Ok(Parsed::Ignored),
        _ => return Err(Bad),
    };
    let results: Vec<&Value> = blocks
        .iter()
        .filter(|block| block.get_str("type") == Some("tool_result"))
        .collect();
    if results.is_empty() {
        return Ok(Parsed::Ignored);
    }
    let success = value.get("toolUseResult").and_then(|result| match result {
        Value::Obj(_) => match result.get("success") {
            Some(Value::Bool(success)) => Some(*success),
            _ => None,
        },
        _ => None,
    });
    for block in &results {
        let id = block.get_str("tool_use_id").ok_or(Bad)?;
        let error = match block.get("is_error") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(error)) => *error,
            Some(_) => return Err(Bad),
        };
        let load = if error {
            Load::NotLoaded
        } else if results.len() == 1 {
            match success {
                Some(true) => Load::Loaded,
                Some(false) => Load::NotLoaded,
                None => Load::Unknown,
            }
        } else {
            Load::Unknown
        };
        out.push(Event::Result {
            id: Key::of(b"call", id.as_bytes()),
            error,
            load: Some(load),
        });
    }
    Ok(Parsed::Kept)
}

/// The (server, tool) of a raw `mcp__S__T` name: exactly one `__` after the
/// prefix and no `___`, both parts valid identities, else both unresolved;
/// `None` for any other name. Never split on a single underscore.
fn mcp(name: &str) -> Option<(Ident, Ident)> {
    let rest = name.strip_prefix("mcp__")?;
    let unresolved = (
        Ident::Unresolved(Kind::Server),
        Ident::Unresolved(Kind::Tool),
    );
    if rest.matches("__").count() != 1 || rest.contains("___") {
        return Some(unresolved);
    }
    let (server, tool) = rest.split_once("__")?;
    match (
        tool_stats::ident(Kind::Server, server),
        tool_stats::ident(Kind::Tool, tool),
    ) {
        (server @ Ident::Name(_), tool @ Ident::Name(_)) => Some((server, tool)),
        _ => Some(unresolved),
    }
}

/// Q1 b: an injected skill body's base-directory line. Only its first text
/// line is looked at, only when it carries [`BASE_PREFIX`].
fn base(value: &Value, roots: &Roots<'_>, out: &mut Vec<Event>) -> Parsed {
    let Some(source) = value.get_str("sourceToolUseID") else {
        return Parsed::Ignored;
    };
    let first = match value
        .get("message")
        .and_then(|message| message.get("content"))
    {
        Some(Value::Str(text)) => Some(text.as_str()),
        Some(Value::Arr(blocks)) => blocks
            .iter()
            .find(|block| block.get_str("type") == Some("text"))
            .and_then(|block| block.get_str("text")),
        _ => None,
    };
    let Some(line) = first.and_then(|text| text.lines().next()) else {
        return Parsed::Ignored;
    };
    let Some(dir) = line.strip_prefix(BASE_PREFIX) else {
        return Parsed::Ignored;
    };
    let Some(dir) = tool_stats::canonical_dir(dir.trim_end_matches('\r')) else {
        return Parsed::Ignored;
    };
    let class = classify(&dir, roots);
    out.push(Event::SkillBase {
        id: Key::of(b"call", source.as_bytes()),
        class,
        resource: Resource::of(Adapter::Claude, class, &dir),
        dir: Dir::of(Adapter::Claude, &dir),
    });
    Parsed::Kept
}

/// Q2 b: a `Read` of `path`, digested. A relative or `..` path never
/// qualifies and gives nothing; one over [`READ_PATH_MAX`] is a diagnostic.
fn read(id: Key, path: &str, roots: &Roots<'_>) -> Option<Event> {
    if path.len() > READ_PATH_MAX {
        return Some(Event::Read {
            id,
            direct: None,
            ancestors: Vec::new(),
            over: true,
        });
    }
    let canonical = tool_stats::canonical_dir(path)?;
    let direct = direct(&canonical, roots);
    let ancestors = tool_stats::ancestors(&canonical)
        .into_iter()
        .map(|dir| Dir::of(Adapter::Claude, dir))
        .collect();
    Some(Event::Read {
        id,
        direct,
        ancestors,
        over: false,
    })
}

/// `<parent>/<d>/SKILL.md` for a seat's `<config home>/skills` (user) or
/// `<work dir>/.claude/skills` (project): the skill `<d>` and its resource.
fn direct(path: &str, roots: &Roots<'_>) -> Option<(Ident, ScopeClass, Resource)> {
    let skill = path.strip_suffix("/SKILL.md")?;
    let (parent, name) = skill.rsplit_once('/')?;
    let class = if homes(roots).any(|home| join(&home, "skills") == parent) {
        ScopeClass::User
    } else if works(roots).any(|work| join(&work, ".claude/skills") == parent) {
        ScopeClass::Project
    } else {
        return None;
    };
    Some((
        tool_stats::ident(Kind::Skill, name),
        class,
        Resource::of(Adapter::Claude, class, skill),
    ))
}

/// Where canonical skill directory `dir` sits among the seat roots.
fn classify(dir: &str, roots: &Roots<'_>) -> ScopeClass {
    if homes(roots).any(|home| tool_stats::inside(dir, &join(&home, "skills"))) {
        ScopeClass::User
    } else if works(roots).any(|work| tool_stats::inside(dir, &join(&work, ".claude/skills"))) {
        ScopeClass::Project
    } else if homes(roots).any(|home| tool_stats::inside(dir, &join(&home, "plugins"))) {
        ScopeClass::Plugin
    } else {
        ScopeClass::Other
    }
}

fn canonical_roots(paths: &[PathBuf]) -> impl Iterator<Item = String> + '_ {
    paths
        .iter()
        .filter_map(|path| path.to_str().and_then(tool_stats::canonical_dir))
}

fn homes<'a>(roots: &Roots<'a>) -> impl Iterator<Item = String> + 'a {
    canonical_roots(roots.config_homes)
}

fn works<'a>(roots: &Roots<'a>) -> impl Iterator<Item = String> + 'a {
    canonical_roots(roots.work_dirs)
}

fn join(dir: &str, rest: &str) -> String {
    if dir == "/" {
        format!("/{rest}")
    } else {
        format!("{dir}/{rest}")
    }
}
