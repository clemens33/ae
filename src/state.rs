//! The `state` helper's WRITE path — the first place the core appends to a
//! session's event container.
//!
//! A declaration is what the frozen `helper_state_main` writes: one `state`
//! event — `{"ts","actor","action":"state","ref":<value>,"summary":<reason>}`
//! — and, for `done`, a second legacy `{"action":"done","summary":<reason>}`
//! line that a watchdog started before the state helper existed still
//! understands. The dual emit stays until every running watchdog has
//! restarted. The no-argument READ form is not here: it stays on the bash
//! body, so `state` with nothing to declare keeps its current answer.
//!
//! # The safety rules this path exists to keep (P2.2 ruling)
//!
//! * **The actor is the calling pane and nothing else.** It is the
//!   [`Viewer`] P2.1b reads from `TMUX_PANE`; there is no `--actor` and no
//!   `human` fallback. An unidentified caller is refused at 1 and writes
//!   nothing — the frozen helper writes `actor:"human"` from any shell, which
//!   is a state nobody declared.
//! * **One lock, the same lock.** `<container>.lock` is held exclusively with
//!   `flock(2)` advisory semantics — the lock `ae_log_append` takes with
//!   `flock -w 5 8` — for at most [`LOCK_WAIT`], and held through the whole
//!   append, so a bash writer and this one never interleave lines.
//! * **No success without the bytes, and no bytes without success.** The
//!   frozen helper prints `Marked …` BEFORE it emits, so its caller can be told
//!   a state it never recorded. This path prints only after the append is
//!   durable (`fdatasync` returned); any lock or append failure is non-zero,
//!   on stderr, with no success line — and the append is a transaction under
//!   the held lock: on failure the container is cut back to its prior length,
//!   so neither a truncated record nor an unacknowledged valid state is left
//!   behind ([`commit`]).
//!
//! # Not a read door
//!
//! `clippy.toml`'s eleven disallowed entry points and the criterion-3
//! inventory over them are about READING the world; `OpenOptions::open` for
//! append is not among them, so this module is deliberately absent from that
//! inventory. What keeps the write conspicuous is that this is the only
//! `OpenOptions` in product code — a second one is a review question.
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::event_text::CONTAINER;
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

/// Parse the helper's argv after the meta directory: `<value> [reason…]`.
///
/// # Errors
///
/// [`Usage`] for a value outside [`VALUES`] or a `blocked` with no reason.
pub fn parse(tail: &[String]) -> Result<Declaration, Usage> {
    let Some((value, rest)) = tail.split_first() else {
        return Err(Usage::UnknownValue(String::new()));
    };
    if !VALUES.contains(&value.as_str()) {
        return Err(Usage::UnknownValue(value.clone()));
    }
    let reason = rest.join(" ");
    if value == "blocked" && reason.is_empty() {
        return Err(Usage::BlockedNeedsReason);
    }
    Ok(Declaration {
        value: value.clone(),
        reason,
    })
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
///
/// The returned handle IS the lock: dropping it releases. `flock(2)` locks
/// belong to the open file description, so a bash `flock` on the same path
/// and this one exclude each other.
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
///
/// `write_all` can fail after a prefix, and a sync can fail after the whole
/// body is down; in both cases the container is cut back to the length it had
/// before, under the lock the caller still holds, so a failed declaration
/// neither leaves a truncated record for the next reader to choke on nor a
/// valid state its caller was told was not recorded. The error reported is
/// the write's.
///
/// **The rollback is itself synced.** A sync can fail AFTER the body reached
/// durable storage; if the cut back to `before` were only in the page cache, a
/// crash would resurrect the rejected record — durable on disk, refused to its
/// caller. So the rollback is truncate THEN sync, and only then is the
/// container "none of them, durably". A rollback whose truncate or sync fails
/// is reported instead of the write's error, naming both and saying the
/// container's state is unknown, because then it really is and the caller
/// must hear that rather than the tidier lie.
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
        Declaration, Failure, LOCK_WAIT, SUMMARY_CAP, Sink, USAGE, Usage, acquire, commit, declare,
        event_body, event_line, parse, summary_of,
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
            Ok(Declaration {
                value: "working".to_owned(),
                reason: "two words".to_owned()
            })
        );
        assert_eq!(
            parse(&words(&["done"])),
            Ok(Declaration {
                value: "done".to_owned(),
                reason: String::new()
            })
        );
        assert_eq!(parse(&words(&["blocked"])), Err(Usage::BlockedNeedsReason));
        assert!(parse(&words(&["blocked", "on x"])).is_ok());
        assert_eq!(
            parse(&words(&["Working"])),
            Err(Usage::UnknownValue("Working".to_owned())),
            "the tokens are exact"
        );
        assert_eq!(parse(&[]), Err(Usage::UnknownValue(String::new())));
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
}
