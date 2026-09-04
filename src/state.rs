//! The `state` helper's WRITE path — the first place the core appends to a
//! session's event container.
//!
//! A declaration is what the frozen `helper_state_main` writes: one `state`
//! event — `{"ts","actor","action":"state","ref":<value>,"summary":<reason>}`
//! — and, for `done`, a second legacy `{"action":"done","summary":<reason>}`
//! line that a watchdog started before the state helper existed still
//! understands. The dual emit stays until every running watchdog has
//! restarted.
//!
//! The no-argument READ form (P2.4) is here too: what `ae_latest_state_for`
//! finds — the newest `{`-prefixed line in the container whose `actor` is the
//! caller and whose `action` is `state` or the legacy `done` — rendered as
//! `<actor> state: <value>[ — <reason>]  (since <ts>)`, or `(none declared)`.
//! Read through [`crate::event_text`]'s frozen primitives, so the reversal,
//! the line filter and the member extraction are the ones `requests` already
//! shares with the bash body. See [`read_line`] for the one deliberate
//! rendering difference.
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::event_text::{self, CONTAINER};
use crate::json::Value;
use crate::requests::Viewer;
use crate::time::Timestamp;

/// How long a declaration waits for the container lock — `flock -w 5`.
pub const LOCK_WAIT: Duration = Duration::from_secs(5);

/// How often the lock is retried while waiting.
const LOCK_POLL: Duration = Duration::from_millis(20);

/// The reason cap, in CHARACTERS — `ae_cap_summary … 200` counts characters
/// under a UTF-8 locale, and so does this.
pub const SUMMARY_CAP: usize = 200;

/// `ae_emit_event`'s chat arm: a `chat` event's summary keeps its newlines and
/// tabs and is capped at this many characters, not [`SUMMARY_CAP`].
pub const CHAT_SUMMARY_CAP: usize = 3500;

/// The four states, exactly as the helper spells them.
pub const VALUES: [&str; 4] = ["working", "waiting-user", "blocked", "done"];

/// The frozen `helper_state_usage` text, byte for byte. Exit 2 goes with it.
pub const USAGE: &str = "Usage: state <working|waiting-user|blocked|done> [reason]\n       state                              # print current state\n\n  working       actively making progress\n  waiting-user  needs human input\n  blocked       stuck on external dep — REASON REQUIRED\n  done          complete or paused\n";

/// The refusal when the caller has no pane identity.
pub const NO_IDENTITY: &str =
    "Error: could not detect current agent identity; declare state from an ae pane";

/// The exit status of every refusal and failure on this path: it went wrong.
pub const EXIT_FAILED: u8 = 1;

/// The exit status of a usage error, shared with the frozen helper.
pub const EXIT_USAGE: u8 = 2;

/// A parsed declaration: the value and the reason as the caller typed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// One of [`VALUES`].
    pub value: String,
    /// The remaining arguments joined by one space — `"${*:-}"`.
    pub reason: String,
}

/// Why argv was refused. Both render to stderr and exit [`EXIT_USAGE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Usage {
    /// `blocked` with no reason — the frozen helper's own error line first.
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
    /// The `ref` of a `state` event, or `done` for a legacy `done` event.
    pub value: Vec<u8>,
    /// The `summary` — empty when the event carries none.
    pub reason: Vec<u8>,
    /// The `ts`.
    pub ts: Vec<u8>,
}

/// Scan the container newest-first and stop at the first line that is
/// `actor`'s `state` (value = `ref`, reason = `summary`) or legacy `done`
/// (value = `done`) event.
///
/// Every step is the frozen body's: `_ae_tac` ([`event_text::reversed`] — a
/// torn last record is glued onto the line before it, not repaired), the
/// `while read` loop over complete lines, the `{`-prefix filter, and
/// `_event_json_str` for each member ([`event_text::extract`] — the FIRST
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
/// declared)`, or `<actor> state: <value>[ — <reason>]  (since <ts>)` — the
/// frozen printf, two spaces before the parenthesis included.
///
/// One deliberate difference. The frozen body hands its three fields to
/// `IFS=$'\t' read -r st reason ts`, and tab is an IFS *whitespace*
/// character: a run of tabs is one delimiter, so an EMPTY reason vanishes and
/// the timestamp slides into its place — a reason-less `working` renders as
/// `working — 2026-…Z  (since )` (measured on the frozen body). This renders
/// the fields as read: `working  (since 2026-…Z)`.
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
    let container = event_text::read_container(&dir.join(CONTAINER));
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

/// The summary as the frozen emitter renders it FOR THIS ACTION — both of
/// `ae_emit_event`'s arms. `chat` keeps its newlines and tabs and is capped at
/// [`CHAT_SUMMARY_CAP`] characters (a `say` line is a paragraph, not a label);
/// every other action is [`summary_of`], flattened and capped at
/// [`SUMMARY_CAP`]. The action is whatever the caller's `_AE_EVENT_ACTION`
/// made it, so a `send` run under `_AE_EVENT_ACTION=chat` takes the chat arm
/// exactly as the bash finisher did. Both cuts land on a character boundary by
/// construction.
#[must_use]
pub fn summary_for(action: &str, text: &str) -> String {
    if action == "chat" {
        text.chars().take(CHAT_SUMMARY_CAP).collect()
    } else {
        summary_of(text)
    }
}

/// One event line, `\n` included, in the frozen emitter's shape and order:
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

/// The bytes one declaration appends: the `state` line, plus the legacy
/// `done` line for `done`.
#[must_use]
pub fn event_body(ts: Timestamp, actor: &str, declaration: &Declaration) -> String {
    let summary = summary_of(&declaration.reason);
    let mut body = event_line(ts, actor, "state", &declaration.value, &summary);
    if declaration.value == "done" {
        body.push_str(&event_line(ts, actor, "done", "", &summary));
    }
    body
}

/// Why a declaration was not recorded. Every variant is [`EXIT_FAILED`].
#[derive(Debug)]
pub enum Failure {
    /// No pane identity — nothing was opened.
    NoIdentity,
    /// The lock was not acquired within [`LOCK_WAIT`], or could not be opened.
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
                LOCK_WAIT.as_secs()
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
    let container = dir.join(CONTAINER);
    match append_locked(&container, body.as_bytes()) {
        Ok(()) => {}
        Err(Locked::Lock(path, why)) => return Err(Failure::Lock(path, why)),
        Err(Locked::Append(path, why)) => return Err(Failure::Append(path, why)),
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

/// Why a locked append did not happen: which step, on which path.
#[derive(Debug)]
pub enum Locked {
    /// The lock file could not be opened or the lock was not acquired within
    /// [`LOCK_WAIT`].
    Lock(String, io::Error),
    /// The append itself failed (and was rolled back where it could be).
    Append(String, io::Error),
}

impl From<Locked> for io::Error {
    fn from(why: Locked) -> Self {
        match why {
            Locked::Lock(path, cause) => Self::new(
                cause.kind(),
                format!(
                    "could not lock {path} within {}s: {cause}",
                    LOCK_WAIT.as_secs()
                ),
            ),
            Locked::Append(path, cause) => {
                Self::new(cause.kind(), format!("could not append to {path}: {cause}"))
            }
        }
    }
}

/// Append `bytes` to `path` under `<path>.lock` — `ae_log_append`, exactly:
/// the lock is the file's own `.lock` sibling, taken with `flock -w 5`, held
/// through the append. The append is the [`commit`] transaction. Shared by
/// the event container and `memo.tsv`, which each keep their own lock.
///
/// # Errors
///
/// [`Locked`] — which step failed, on which path.
pub fn append_locked(path: &Path, bytes: &[u8]) -> Result<(), Locked> {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock_path = Path::new(&lock_path);
    let _held = acquire(lock_path, LOCK_WAIT)
        .map_err(|why| Locked::Lock(lock_path.display().to_string(), why))?;
    append(path, bytes).map_err(|why| Locked::Append(path.display().to_string(), why))
}

/// Append one event line to `dir`'s container under its lock.
///
/// # Errors
///
/// The [`Locked`] failure, flattened to one [`io::Error`] naming the step.
pub fn emit(dir: &Path, line: &str) -> io::Result<()> {
    append_locked(&dir.join(CONTAINER), line.as_bytes()).map_err(io::Error::from)
}

/// Take the exclusive advisory lock on `path`, retrying for up to `wait`.
pub(crate) fn acquire(path: &Path, wait: Duration) -> io::Result<File> {
    let file = OpenOptions::new().append(true).create(true).open(path)?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                if started.elapsed() >= wait {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "another writer holds the lock",
                    ));
                }
                std::thread::sleep(LOCK_POLL);
            }
            Err(TryLockError::Error(why)) => return Err(why),
        }
    }
}

/// Append `bytes` to `path`, creating it, as one transaction under the lock
/// the caller holds.
fn append(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    commit(&mut file, bytes)
}

/// What a transactional append needs from its container. [`File`] is the
/// real one; the test double is how the failure arms are driven, because a
/// real write that fails after a prefix cannot be arranged on demand.
trait Sink {
    /// The current length — the point to roll back to.
    fn len(&mut self) -> io::Result<u64>;
    /// Write all of `bytes`, possibly failing after a prefix.
    fn put(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Make what was written durable — `fdatasync`.
    fn sync(&mut self) -> io::Result<()>;
    /// Cut the container back to `len`.
    fn truncate(&mut self, len: u64) -> io::Result<()>;
}

impl Sink for File {
    fn len(&mut self) -> io::Result<u64> {
        self.metadata().map(|meta| meta.len())
    }
    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write_all(bytes)
    }
    fn sync(&mut self) -> io::Result<()> {
        self.sync_data()
    }
    fn truncate(&mut self, len: u64) -> io::Result<()> {
        self.set_len(len)
    }
}

/// Write `bytes` so that afterwards the container holds either all of them,
/// durably, or none of them.
fn commit(sink: &mut impl Sink, bytes: &[u8]) -> io::Result<()> {
    let before = sink.len()?;
    match sink.put(bytes).and_then(|()| sink.sync()) {
        Ok(()) => Ok(()),
        Err(failed) => match sink.truncate(before).and_then(|()| sink.sync()) {
            Ok(()) => Err(failed),
            Err(rollback) => Err(io::Error::new(
                rollback.kind(),
                format!(
                    "{failed}; and rolling the container back to {before} bytes failed: {rollback} \
                     — the container's state is UNKNOWN"
                ),
            )),
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the door wrote; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::{
        CHAT_SUMMARY_CAP, Command, Declaration, Failure, LOCK_WAIT, Latest, SUMMARY_CAP, Sink,
        USAGE, Usage, acquire, commit, declare, event_body, event_line, latest, parse, read,
        read_line, summary_for, summary_of,
    };
    use crate::requests::Viewer;
    use crate::time::Timestamp;
    use std::path::PathBuf;
    use std::time::Duration;

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

    /// A container that fails on demand: after `fail_after` bytes of a put, or
    /// at sync, or at truncate. Records what was asked of it.
    #[derive(Default)]
    struct Flaky {
        held: Vec<u8>,
        fail_put_after: Option<usize>,
        /// Per sync call, in order: `true` fails that call.
        fail_syncs: Vec<bool>,
        /// How many syncs were asked for.
        syncs: usize,
        fail_truncate: bool,
        truncated_to: Vec<u64>,
    }

    impl Sink for Flaky {
        fn len(&mut self) -> std::io::Result<u64> {
            Ok(self.held.len() as u64)
        }
        fn put(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            if let Some(prefix) = self.fail_put_after {
                self.held
                    .extend_from_slice(&bytes[..prefix.min(bytes.len())]);
                return Err(std::io::Error::other("disk full after a prefix"));
            }
            self.held.extend_from_slice(bytes);
            Ok(())
        }
        fn sync(&mut self) -> std::io::Result<()> {
            let call = self.syncs;
            self.syncs += 1;
            if self.fail_syncs.get(call).copied().unwrap_or(false) {
                return Err(std::io::Error::other(format!("sync {} failed", call + 1)));
            }
            Ok(())
        }
        fn truncate(&mut self, len: u64) -> std::io::Result<()> {
            self.truncated_to.push(len);
            if self.fail_truncate {
                return Err(std::io::Error::other("truncate failed"));
            }
            self.held.truncate(usize::try_from(len).unwrap());
            Ok(())
        }
    }

    #[test]
    fn a_write_that_fails_after_a_prefix_rolls_the_container_back() {
        let mut sink = Flaky {
            held: b"{\"ts\":\"earlier\"}\n".to_vec(),
            fail_put_after: Some(7),
            ..Flaky::default()
        };
        let before = sink.held.clone();
        let result = commit(&mut sink, b"{\"ts\":\"now\",\"action\":\"state\"}\n");
        assert!(result.is_err());
        assert_eq!(
            sink.held, before,
            "not one byte of the failed record survives"
        );
        assert_eq!(sink.truncated_to, vec![before.len() as u64]);
    }

    #[test]
    fn a_sync_that_fails_after_a_complete_write_rolls_the_container_back_too() {
        // The subtler arm: the bytes are all there, the caller is told "not
        // recorded", and without the rollback the next reader would find a
        // state nobody acknowledged.
        let mut sink = Flaky {
            fail_syncs: vec![true],
            ..Flaky::default()
        };
        let result = commit(&mut sink, b"{\"action\":\"state\"}\n");
        assert_eq!(
            result.unwrap_err().to_string(),
            "sync 1 failed",
            "the write's error"
        );
        assert!(sink.held.is_empty());
        assert_eq!(sink.truncated_to, vec![0]);
        assert_eq!(sink.syncs, 2, "the rollback was synced too");
    }

    #[test]
    fn a_rollback_whose_own_sync_fails_is_reported_as_an_unknown_state() {
        // The body reached durable storage, the sync after it failed, the cut
        // back succeeded in the page cache — and THAT sync failed. Nothing can
        // now be said about what a crash would leave, so nothing is.
        let mut sink = Flaky {
            fail_syncs: vec![true, true],
            ..Flaky::default()
        };
        let why = commit(&mut sink, b"{\"action\":\"state\"}\n")
            .unwrap_err()
            .to_string();
        assert!(why.contains("sync 1 failed"), "{why}");
        assert!(
            why.contains("rolling the container back to 0 bytes failed: sync 2 failed"),
            "{why}"
        );
        assert!(why.contains("UNKNOWN"), "{why}");
        assert_eq!(sink.syncs, 2);
    }

    #[test]
    fn a_rollback_that_fails_is_what_gets_reported() {
        let mut sink = Flaky {
            fail_put_after: Some(2),
            fail_truncate: true,
            ..Flaky::default()
        };
        let why = commit(&mut sink, b"abcdef").unwrap_err().to_string();
        assert!(why.contains("disk full after a prefix"), "{why}");
        assert!(
            why.contains("rolling the container back to 0 bytes failed"),
            "{why}"
        );
        assert!(why.contains("UNKNOWN"), "{why}");
        assert_eq!(sink.syncs, 0, "a failed truncate is not followed by a sync");
    }

    #[test]
    fn a_successful_commit_never_truncates() {
        let mut sink = Flaky::default();
        commit(&mut sink, b"line\n").unwrap();
        assert_eq!(sink.held, b"line\n");
        assert!(sink.truncated_to.is_empty());
        assert_eq!(sink.syncs, 1, "one sync, for the write");
    }

    #[test]
    fn a_held_lock_fails_the_declaration_at_the_bound_with_no_bytes_written() {
        let dir = scratch("held");
        let lock_path = dir.join("events.jsonl.lock");
        // Another open file description holding the same flock: exactly what a
        // bash `flock 8` on the same path is.
        let holder = acquire(&lock_path, Duration::from_millis(10)).unwrap();
        let started = std::time::Instant::now();
        let waited = acquire(&lock_path, Duration::from_millis(150));
        assert!(waited.is_err(), "the lock is held");
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "the bound was honoured"
        );
        drop(holder);
        assert!(
            acquire(&lock_path, Duration::from_millis(10)).is_ok(),
            "released"
        );
        // The real path uses the real bound: 5s, per flock -w 5.
        assert_eq!(LOCK_WAIT, Duration::from_secs(5));
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
        // first and runs into the line before it. The bash body then reads the
        // FIRST `"actor":"` on that glued line — the remainder's — so a torn
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
