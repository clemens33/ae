//! The `state` helper's WRITE path — the first place the core appends to a
//! session's event container.
//!
//! A declaration writes one `state` event —
//! `{"ts","actor","action":"state","ref":<value>,"summary":<reason>}` — and,
//! for `done`, a second `{"action":"done","summary":<reason>}` line that an
//! older watchdog still understands. The dual emit stays until every running
//! watchdog has restarted.
//!
//! The no-argument READ form is here too: the newest `{`-prefixed line in the
//! container whose `actor` is the caller and whose `action` is `state` or a
//! bare `done`, rendered as `<actor> state: <value>[ — <reason>]  (since
//! <ts>)`, or `(none declared)`. Read through [`crate::event_text`], so the
//! reversal, the line filter and the member extraction are the ones `requests`
//! shares.
use std::io;
use std::path::Path;

use crate::event_text;
use crate::json::Value;
use crate::requests::Viewer;
use crate::store;
use crate::time::Timestamp;

/// The reason cap, in CHARACTERS — `ae_cap_summary … 200` counts characters
/// under a UTF-8 locale, and so does this.
pub const SUMMARY_CAP: usize = 200;

/// `ae_emit_event`'s chat arm: a `chat` event's summary keeps its newlines and
/// tabs and is capped at this many characters, not [`SUMMARY_CAP`].
pub const CHAT_SUMMARY_CAP: usize = 3500;

/// The four states, exactly as the helper spells them.
pub const VALUES: [&str; 4] = ["working", "waiting-user", "blocked", "done"];

/// The usage text.
pub const USAGE: &str = "Usage: state <working|waiting-user|blocked|done> [reason]\n       state                              # print current state\n\n  working       actively making progress\n  waiting-user  needs human input\n  blocked       stuck on external dep — REASON REQUIRED\n  done          complete or paused\n";

/// The refusal when the caller has no pane identity.
pub const NO_IDENTITY: &str =
    "Error: could not detect current agent identity; declare state from an ae pane";

/// The exit status of every refusal and failure on this path: it went wrong.
pub const EXIT_FAILED: u8 = 1;

/// The exit status of a usage error.
pub const EXIT_USAGE: u8 = 2;

/// A parsed declaration: the value and the reason as the caller typed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// One of [`VALUES`].
    pub value: String,
    /// The remaining arguments joined by one space — `"${*:-}"`.
    pub reason: String,
}

/// Why argv was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Usage {
    /// `blocked` with no reason.
    BlockedNeedsReason,
    /// Not one of [`VALUES`].
    UnknownValue(String),
}

impl Usage {
    /// The stderr text: the helper's error line where it prints one, then
    /// [`USAGE`].
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::BlockedNeedsReason => format!("Error: 'blocked' requires a reason\n{USAGE}"),
            Self::UnknownValue(_) => USAGE.to_owned(),
        }
    }
}

/// What the helper's argv asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `state` with nothing after the meta directory — print the caller's
    /// latest declaration.
    Read,
    /// `state <value> [reason…]`.
    Declare(Declaration),
}

/// Parse the helper's argv after the meta directory: nothing, or
/// `<value> [reason…]`.
///
/// # Errors
///
/// [`Usage`] for a value outside [`VALUES`] or a `blocked` with no reason.
pub fn parse(tail: &[String]) -> Result<Command, Usage> {
    let Some((value, rest)) = tail.split_first() else {
        return Ok(Command::Read);
    };
    if !VALUES.contains(&value.as_str()) {
        return Err(Usage::UnknownValue(value.clone()));
    }
    let reason = rest.join(" ");
    if value == "blocked" && reason.is_empty() {
        return Err(Usage::BlockedNeedsReason);
    }
    Ok(Command::Declare(Declaration {
        value: value.clone(),
        reason,
    }))
}

/// The newest declaration an actor made, as `ae_latest_state_for` finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Latest {
    /// The `ref` of a `state` event, or `done` for a bare `done` event.
    pub value: Vec<u8>,
    /// The `summary` — empty when the event carries none.
    pub reason: Vec<u8>,
    /// The `ts`.
    pub ts: Vec<u8>,
}

/// Scan the container newest-first and stop at the first line that is
/// `actor`'s `state` (value = `ref`, reason = `summary`) or bare `done`
/// (value = `done`) event.
///
/// The steps: [`event_text::reversed`] (a torn last record is glued onto the
/// line before it, not repaired), the walk over complete lines, the
/// `{`-prefix filter, and [`event_text::extract`] for each member (the FIRST
/// occurrence of the key, unescaped the emitter's way). Another action by the
/// same actor is skipped, not a stop.
///
/// ```
/// use ae::state::latest;
///
/// let container = concat!(
///     r#"{"ts":"2026-08-27T08:00:00Z","actor":"cl:lead","action":"state","ref":"working","summary":"on it"}"#, "\n",
///     r#"{"ts":"2026-08-27T08:00:01Z","actor":"cl:lead","action":"ask","ref":"ae-1"}"#, "\n",
///     r#"{"ts":"2026-08-27T08:00:02Z","actor":"cl:other","action":"state","ref":"blocked","summary":"x"}"#, "\n",
/// );
/// let found = latest(container.as_bytes(), "cl:lead").unwrap();
/// assert_eq!(found.value, b"working");
/// assert_eq!(found.reason, b"on it");
/// assert_eq!(found.ts, b"2026-08-27T08:00:00Z");
/// assert!(latest(container.as_bytes(), "cl:nobody").is_none());
/// ```
#[must_use]
pub fn latest(container: &[u8], actor: &str) -> Option<Latest> {
    let stream = event_text::reversed(container);
    for line in event_text::read_lines(&stream) {
        let Some(line) = event_text::event_line(line) else {
            continue;
        };
        if event_text::extract(line, "actor") != actor.as_bytes() {
            continue;
        }
        match event_text::extract(line, "action").as_slice() {
            b"state" => {
                return Some(Latest {
                    value: event_text::extract(line, "ref"),
                    reason: event_text::extract(line, "summary"),
                    ts: event_text::extract(line, "ts"),
                });
            }
            b"done" => {
                return Some(Latest {
                    value: b"done".to_vec(),
                    reason: event_text::extract(line, "summary"),
                    ts: event_text::extract(line, "ts"),
                });
            }
            _ => {}
        }
    }
    None
}

/// The stdout of `state` with nothing to declare: `<actor> state: (none
/// declared)`, or `<actor> state: <value>[ — <reason>]  (since <ts>)`, two
/// spaces before the parenthesis included.
///
/// An EMPTY reason keeps its place rather than letting the timestamp slide
/// into it: a reason-less `working` renders `working  (since 2026-…Z)`.
///
/// ```
/// use ae::state::{Latest, read_line};
///
/// assert_eq!(read_line("cl:lead", None), b"cl:lead state: (none declared)\n");
/// let bare = Latest { value: b"working".to_vec(), reason: Vec::new(), ts: b"2026-08-27T08:00:00Z".to_vec() };
/// assert_eq!(
///     read_line("cl:lead", Some(&bare)),
///     "cl:lead state: working  (since 2026-08-27T08:00:00Z)\n".as_bytes()
/// );
/// ```
#[must_use]
pub fn read_line(actor: &str, latest: Option<&Latest>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(actor.as_bytes());
    out.extend_from_slice(b" state: ");
    match latest {
        None => out.extend_from_slice(b"(none declared)"),
        Some(found) => {
            out.extend_from_slice(&found.value);
            if !found.reason.is_empty() {
                out.extend_from_slice(" — ".as_bytes());
                out.extend_from_slice(&found.reason);
            }
            out.extend_from_slice(b"  (since ");
            out.extend_from_slice(&found.ts);
            out.push(b')');
        }
    }
    out.push(b'\n');
    out
}

/// `state` with nothing to declare, for `viewer`.
#[must_use]
pub fn read(dir: &Path, viewer: &Viewer) -> Vec<u8> {
    let actor = if viewer.is_known() {
        viewer.display.as_str()
    } else {
        "human"
    };
    let container = store::open(dir).container();
    read_line(actor, latest(&container, actor).as_ref())
}

/// The reason as the event carries it: newlines and tabs flattened to spaces,
/// then capped at [`SUMMARY_CAP`] characters — `ae_emit_event`'s non-chat arm.
#[must_use]
pub fn summary_of(reason: &str) -> String {
    reason
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .take(SUMMARY_CAP)
        .collect()
}

/// The summary as it is rendered FOR THIS ACTION.
#[must_use]
pub fn summary_for(action: &str, text: &str) -> String {
    if action == "chat" {
        text.chars().take(CHAT_SUMMARY_CAP).collect()
    } else {
        summary_of(text)
    }
}

/// One event line, `\n` included, in the emitter's shape and order:
/// `ts`, `actor`, `action`, then `ref` and `summary` only when non-empty.
#[must_use]
pub fn event_line(
    ts: Timestamp,
    actor: &str,
    action: &str,
    reference: &str,
    summary: &str,
) -> String {
    let mut fields = vec![
        ("ts".to_owned(), Value::Str(ts.to_string())),
        ("actor".to_owned(), Value::Str(actor.to_owned())),
        ("action".to_owned(), Value::Str(action.to_owned())),
    ];
    if !reference.is_empty() {
        fields.push(("ref".to_owned(), Value::Str(reference.to_owned())));
    }
    if !summary.is_empty() {
        fields.push(("summary".to_owned(), Value::Str(summary.to_owned())));
    }
    let mut line = Value::Obj(fields).render();
    line.push('\n');
    line
}

/// The bytes one declaration appends: the `state` line, plus the bare `done`
/// line for `done`.
#[must_use]
pub fn event_body(ts: Timestamp, actor: &str, declaration: &Declaration) -> String {
    let summary = summary_of(&declaration.reason);
    let mut body = event_line(ts, actor, "state", &declaration.value, &summary);
    if declaration.value == "done" {
        body.push_str(&event_line(ts, actor, "done", "", &summary));
    }
    body
}

/// Why a declaration was not recorded.
#[derive(Debug)]
pub enum Failure {
    /// No pane identity — nothing was opened.
    NoIdentity,
    /// The lock was not acquired within [`crate::store::LOCK_WAIT`], or could not be opened.
    Lock(String, io::Error),
    /// The container could not be opened or the write did not complete.
    Append(String, io::Error),
}

impl Failure {
    /// The stderr line.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NoIdentity => NO_IDENTITY.to_owned(),
            Self::Lock(path, why) => format!(
                "ae: state not recorded: could not lock {path} within {}s: {why}",
                store::LOCK_WAIT.as_secs()
            ),
            Self::Append(path, why) => {
                format!("ae: state not recorded: could not append to {path}: {why}")
            }
        }
    }
}

/// Record `declaration` for `viewer` in `dir`'s container, and return the
/// success line for stdout — only once the bytes are down.
///
/// # Errors
///
/// [`Failure`] — see its variants. Nothing is written on any of them.
pub fn declare(
    dir: &Path,
    viewer: &Viewer,
    declaration: &Declaration,
    now: Timestamp,
) -> Result<String, Failure> {
    if !viewer.is_known() {
        return Err(Failure::NoIdentity);
    }
    let body = event_body(now, &viewer.display, declaration);
    match store::open(dir).append_event(&body) {
        Ok(()) => {}
        Err(store::Error::Lock(path, why)) => return Err(Failure::Lock(path, why)),
        Err(store::Error::Append(path, why)) => return Err(Failure::Append(path, why)),
    }
    let reason = if declaration.reason.is_empty() {
        String::new()
    } else {
        format!(": {}", declaration.reason)
    };
    Ok(format!(
        "Marked {} {}{reason}\n",
        viewer.display, declaration.value
    ))
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the door wrote; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::{
        CHAT_SUMMARY_CAP, Command, Declaration, Failure, Latest, SUMMARY_CAP, USAGE, Usage,
        declare, event_body, event_line, latest, parse, read, read_line, summary_for, summary_of,
    };
    use crate::requests::Viewer;
    use crate::time::Timestamp;
    use std::path::PathBuf;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/ae-state-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn lead() -> Viewer {
        Viewer {
            slot: "main".to_owned(),
            session: "s".to_owned(),
            display: "cl:lead".to_owned(),
        }
    }

    #[test]
    fn argv_parses_the_way_the_helper_reads_it() {
        assert_eq!(
            parse(&words(&["working", "two", "words"])),
            Ok(Command::Declare(Declaration {
                value: "working".to_owned(),
                reason: "two words".to_owned()
            }))
        );
        assert_eq!(
            parse(&words(&["done"])),
            Ok(Command::Declare(Declaration {
                value: "done".to_owned(),
                reason: String::new()
            }))
        );
        assert_eq!(parse(&words(&["blocked"])), Err(Usage::BlockedNeedsReason));
        assert!(parse(&words(&["blocked", "on x"])).is_ok());
        assert_eq!(
            parse(&words(&["Working"])),
            Err(Usage::UnknownValue("Working".to_owned())),
            "the tokens are exact"
        );
        assert_eq!(
            parse(&[]),
            Ok(Command::Read),
            "nothing to declare is a read"
        );
        assert!(
            Usage::BlockedNeedsReason
                .render()
                .starts_with("Error: 'blocked' requires a reason\n")
        );
        assert!(Usage::BlockedNeedsReason.render().ends_with(USAGE));
        assert_eq!(Usage::UnknownValue("x".to_owned()).render(), USAGE);
    }

    #[test]
    fn the_summary_is_flattened_then_capped_in_characters() {
        assert_eq!(summary_of("a\nb\tc"), "a b c");
        let long: String = "é".repeat(SUMMARY_CAP + 5);
        let capped = summary_of(&long);
        assert_eq!(capped.chars().count(), SUMMARY_CAP);
        assert_eq!(capped.len(), SUMMARY_CAP * 2, "cut on a character boundary");
    }

    #[test]
    fn the_chat_arm_keeps_lines_and_tabs_and_caps_at_its_own_length() {
        assert_eq!(summary_for("chat", "a\nb\tc"), "a\nb\tc");
        assert_eq!(
            summary_for("send", "a\nb\tc"),
            "a b c",
            "every other action is the flattened arm"
        );
        assert_eq!(
            summary_for("say", &"x".repeat(250)).len(),
            SUMMARY_CAP,
            "the literal action chat, nothing that resembles it"
        );
        let long: String = "é\n".repeat(CHAT_SUMMARY_CAP);
        let capped = summary_for("chat", &long);
        assert_eq!(capped.chars().count(), CHAT_SUMMARY_CAP);
        assert!(
            capped.ends_with("é\n"),
            "cut on a character boundary, lines kept"
        );
    }

    #[test]
    fn the_event_line_has_the_frozen_emitter_s_shape_and_order() {
        let ts = Timestamp::parse("2026-08-26T13:00:00Z").unwrap();
        assert_eq!(
            event_line(ts, "cl:lead", "state", "working", "on it"),
            "{\"ts\":\"2026-08-26T13:00:00Z\",\"actor\":\"cl:lead\",\"action\":\"state\",\"ref\":\"working\",\"summary\":\"on it\"}\n"
        );
        // Empty ref and summary are ABSENT members, as `[[ -n … ]] && json+=`
        // makes them — not empty strings.
        assert_eq!(
            event_line(ts, "cl:lead", "done", "", ""),
            "{\"ts\":\"2026-08-26T13:00:00Z\",\"actor\":\"cl:lead\",\"action\":\"done\"}\n"
        );
        // A quote in the reason is escaped, not a second member.
        assert!(event_line(ts, "a", "state", "done", "say \"hi\"").contains("say \\\"hi\\\""));
    }

    #[test]
    fn done_writes_the_legacy_line_too_and_nothing_else_does() {
        let ts = Timestamp::parse("2026-08-26T13:00:00Z").unwrap();
        let done = Declaration {
            value: "done".to_owned(),
            reason: "fin".to_owned(),
        };
        let body = event_body(ts, "cl:lead", &done);
        assert_eq!(body.lines().count(), 2);
        assert!(body.lines().nth(1).unwrap().contains("\"action\":\"done\""));
        let working = Declaration {
            value: "working".to_owned(),
            reason: String::new(),
        };
        assert_eq!(event_body(ts, "cl:lead", &working).lines().count(), 1);
    }

    #[test]
    fn an_unidentified_caller_is_refused_and_nothing_is_touched() {
        let dir = scratch("noid");
        let decl = Declaration {
            value: "working".to_owned(),
            reason: String::new(),
        };
        let result = declare(&dir, &Viewer::default(), &decl, Timestamp::now());
        assert!(matches!(result, Err(Failure::NoIdentity)));
        assert!(
            std::fs::read_dir(&dir).unwrap().next().is_none(),
            "no lock, no container"
        );
    }

    #[test]
    fn a_declaration_appends_under_the_lock_and_reports_only_afterwards() {
        let dir = scratch("write");
        let decl = Declaration {
            value: "done".to_owned(),
            reason: "all green".to_owned(),
        };
        let line = declare(&dir, &lead(), &decl, Timestamp::now()).unwrap();
        assert_eq!(line, "Marked cl:lead done: all green\n");
        let container = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        assert_eq!(container.lines().count(), 2);
        assert!(container.starts_with("{\"ts\":\""));
        assert!(
            dir.join("events.jsonl.lock").exists(),
            "the same lock file bash takes"
        );
        // A second declaration APPENDS; the first is untouched.
        let again = Declaration {
            value: "working".to_owned(),
            reason: String::new(),
        };
        declare(&dir, &lead(), &again, Timestamp::now()).unwrap();
        let container = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        assert_eq!(container.lines().count(), 3);
        assert!(
            container
                .lines()
                .last()
                .unwrap()
                .contains("\"ref\":\"working\"")
        );
    }

    fn container(lines: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for line in lines {
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
        }
        out
    }

    #[test]
    fn the_newest_state_or_legacy_done_of_the_actor_wins_and_other_lines_are_skipped() {
        let body = container(&[
            r#"{"ts":"t1","actor":"cl:lead","action":"state","ref":"blocked","summary":"old"}"#,
            r#"{"ts":"t2","actor":"cl:lead","action":"done","summary":"legacy"}"#,
            r#"{"ts":"t3","actor":"cl:other","action":"state","ref":"working","summary":"not mine"}"#,
            r#"{"ts":"t4","actor":"cl:lead","action":"ask","ref":"ae-1","summary":"skipped, not a stop"}"#,
            r#"not an event line, though it names "actor":"cl:lead","action":"state","ref":"done""#,
        ]);
        assert_eq!(
            latest(&body, "cl:lead"),
            Some(Latest {
                value: b"done".to_vec(),
                reason: b"legacy".to_vec(),
                ts: b"t2".to_vec()
            }),
            "the legacy done line is a done state, and it is the newest of the actor's"
        );
        assert_eq!(
            latest(&body, "cl:other").map(|found| found.value),
            Some(b"working".to_vec())
        );
        assert_eq!(latest(&body, "human"), None);
        assert_eq!(latest(b"", "cl:lead"), None);
    }

    #[test]
    fn a_torn_last_record_is_read_glued_the_way_tac_hands_it_over() {
        // `_ae_tac` does not invent a newline: the unterminated remainder lands
        // first and runs into the line before it.
        let mut body = container(&[
            r#"{"ts":"t1","actor":"cl:lead","action":"state","ref":"working","summary":"whole"}"#,
        ]);
        body.extend_from_slice(br#"{"ts":"t2","actor":"cl:lead","action":"state","ref":"done""#);
        let found = latest(&body, "cl:lead").expect("the glued line still names the actor");
        assert_eq!(found.value, b"done");
        assert_eq!(found.ts, b"t2");
        assert_eq!(
            found.reason, b"whole",
            "the summary is the glued-on previous line's"
        );
    }

    #[test]
    fn the_line_is_the_frozen_printf_with_the_fields_as_read() {
        let full = Latest {
            value: b"blocked".to_vec(),
            reason: b"on the lock".to_vec(),
            ts: b"2026-08-27T08:00:00Z".to_vec(),
        };
        assert_eq!(
            read_line("cl:lead", Some(&full)),
            "cl:lead state: blocked — on the lock  (since 2026-08-27T08:00:00Z)\n".as_bytes()
        );
        assert_eq!(
            read_line("@other:cl:lead", None),
            b"@other:cl:lead state: (none declared)\n"
        );
    }

    #[test]
    fn read_asks_for_the_pane_or_for_human_and_treats_no_container_as_none() {
        let dir = PathBuf::from(format!("/tmp/ae-state-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            read(&dir, &Viewer::default()),
            b"human state: (none declared)\n"
        );
        std::fs::write(
            dir.join("events.jsonl"),
            container(&[
                r#"{"ts":"t1","actor":"human","action":"state","ref":"working"}"#,
                r#"{"ts":"t2","actor":"cl:lead","action":"state","ref":"done","summary":"shipped"}"#,
            ]),
        )
        .unwrap();
        assert_eq!(
            read(&dir, &Viewer::default()),
            b"human state: working  (since t1)\n",
            "a reason-less declaration keeps its timestamp where it belongs"
        );
        assert_eq!(
            read(&dir, &lead()),
            "cl:lead state: done — shipped  (since t2)\n".as_bytes()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
