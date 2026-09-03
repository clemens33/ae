//! The oversize-body NOTICE: what crosses the pane when the message will not.
//!
//! Ported from `ae`'s `_notice_compose`, `_notice_prepare`,
//! `_notice_reconstruct` and `_notice_prove`.
//!
//! A framed body over [`LIMIT`] bytes is not pasted. The sender writes it to
//! its own `messages/*.txt` and pastes a POINTER of at most [`NOTICE_CAP`]
//! bytes instead — and because a pointer is a delivery instruction rather than
//! the delivery itself, Enter is FORBIDDEN until the pane's visible input rows
//! prove the exact staged bytes. That proof is [`prove`], and it is the reason
//! [`reconstruct`] exists: the two modelled TUIs wrap a long row with a
//! two-space continuation indent, and `capture-pane` drops the visual wrap
//! space at the row boundary, so the rows have to be rejoined before they can
//! be compared byte for byte.
//!
//! The rejoin reinserts exactly ONE space, and only where the INTENDED bytes
//! prove one belongs. No other whitespace is normalised: this is a
//! byte-preserving proof, not a pretty-printer.

use std::path::Path;

use super::region::Tool;

/// The framed-body size above which a notice replaces the paste.
pub const LIMIT: u64 = 8192;

/// The largest notice that may be treated as a small pointer. Above it the
/// compose REFUSES before any paste, and the full body stays recoverable where
/// the sender wrote it.
pub const NOTICE_CAP: usize = 300;

/// What the paste will carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// The framed body itself.
    Direct,
    /// A pointer to the body file, which must be PROVEN on screen before
    /// Enter.
    Notice(String),
}

/// A notice that could not be composed as a small pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused;

/// Decide the mode for this delivery — `_notice_prepare`.
///
/// Only the two modelled tools get a notice: an unmodelled TUI has no input
/// sensor, so the on-screen proof a notice depends on cannot be taken, and
/// pasting the whole body is what it always got.
///
/// `bytes` is the framed body's own length, not a `wc -c` of the file the
/// frozen helper measured. They are the same number — the body store writes
/// those exact bytes and nothing else — and taking it from the value in hand
/// keeps a delivery decision from depending on a file read that can fail for
/// reasons that have nothing to do with the message.
///
/// # Errors
///
/// [`Refused`]: the composed pointer would exceed [`NOTICE_CAP`], or the body
/// file is not one this composer can name. Refuses BEFORE any paste.
#[allow(
    clippy::too_many_arguments,
    reason = "the frozen preparer's inputs, spelled out rather than bundled"
)]
pub fn prepare(
    tool: Tool,
    action: &str,
    reference: &str,
    actor: &str,
    target_session: &str,
    own_session: &str,
    body_file: &Path,
    bytes: u64,
    reply_dir: &Path,
) -> Result<Mode, Refused> {
    if !tool.is_modelled() || bytes <= LIMIT {
        return Ok(Mode::Direct);
    }
    compose(
        action,
        reference,
        actor,
        body_file,
        bytes,
        target_session,
        own_session,
        reply_dir,
    )
    .map(Mode::Notice)
    .ok_or(Refused)
}

/// Compose the pointer that replaces an oversized framed body —
/// `_notice_compose`.
///
/// The body file is sender-owned; only its PATH crosses the pane. A
/// same-session recipient gets the session-relative `messages/*.txt` grammar
/// and its own reply helper; a cross-session one gets the absolute sender path
/// and the sender's reply helper, because `messages/` would name a different
/// directory in the recipient's session.
#[allow(
    clippy::too_many_arguments,
    reason = "the frozen composer's inputs, spelled out rather than bundled"
)]
#[must_use]
pub fn compose(
    action: &str,
    reference: &str,
    actor: &str,
    body_file: &Path,
    bytes: u64,
    target_session: &str,
    own_session: &str,
    reply_dir: &Path,
) -> Option<String> {
    let name = body_file.file_name()?.to_str()?;
    // The store publishes `<stem>.<action>.<unique>.txt` and nothing else, so
    // this is a check on ae's own writer, not a user-supplied filename — the
    // case-insensitive comparison clippy suggests would accept a name this
    // path can never produce.
    if std::path::Path::new(name)
        .extension()
        .is_none_or(|ext| ext != "txt")
    {
        return None;
    }
    let same_session = !own_session.is_empty() && target_session == own_session;
    let path = if same_session {
        format!("messages/{name}")
    } else {
        body_file.display().to_string()
    };
    let notice = match action {
        "spawn" => format!(
            "⟦ae:task⟧[{reference}] LONG BODY {bytes} B in your session dir: {path} — read it fully, then begin ⟧{reference}⟧"
        ),
        "ask" | "review" | "reply" => {
            if reference.is_empty() {
                return None;
            }
            if same_session {
                format!(
                    "⟦ae:msg from {actor}⟧[{reference}] LONG BODY {bytes} B in your session dir: {path} — read it first; then run your reply helper: reply {reference} ⟧{reference}⟧"
                )
            } else {
                format!(
                    "⟦ae:msg from {actor}⟧[{reference}] LONG BODY {bytes} B: {path} — read it first; reply: {}/reply {reference} ⟧{reference}⟧",
                    reply_dir.display()
                )
            }
        }
        _ => format!(
            "⟦ae:msg from {actor}⟧[-] LONG BODY {bytes} B in your session dir: {path} — read it first ⟧-⟧"
        ),
    };
    (notice.len() <= NOTICE_CAP).then_some(notice)
}

/// Rejoin a one-line notice out of a captured input box — `_notice_reconstruct`.
///
/// Returns `None` when no prompt row carrying the notice's head was found.
#[must_use]
pub fn reconstruct(tool: Tool, capture: &str, intended: &str) -> Option<String> {
    let rows: Vec<String> = capture.lines().map(strip_csi).collect();
    // The head includes the request id, which disambiguates a whole-pane
    // capture carrying several historical notices in its transcript. Message
    // and spawn heads are deliberately distinct: a spawn is an instruction, not
    // transcript chat.
    let head = head_of(intended);
    if head != GENERIC_HEAD && !head_is_wellformed(&head) {
        return None;
    }
    let ornament = match tool {
        Tool::Claude => '❯',
        Tool::Codex => '›',
        Tool::Other => return None,
    };
    for (index, row) in rows.iter().enumerate() {
        let Some(at) = row.find(&head) else { continue };
        let prefix = &row[..at];
        let Some(orn_at) = prefix.find(ornament) else {
            continue;
        };
        // A transcript line may contain the same bytes, but it is not a prompt
        // row: everything before the ornament must be blank.
        if !super::region::is_blank(&prefix[..orn_at]) {
            continue;
        }
        let line = &row[orn_at + ornament.len_utf8()..];
        let line = line
            .strip_prefix(' ')
            .or_else(|| line.strip_prefix('\u{a0}'))
            .unwrap_or(line);
        let mut candidate = line.to_owned();
        for rest in &rows[index + 1..] {
            let rest = rest.trim_end_matches(' ');
            if tool == Tool::Codex && super::region::is_blank(rest) {
                break;
            }
            if tool == Tool::Claude && is_rule(rest) {
                break;
            }
            let Some(part) = rest.strip_prefix("  ") else {
                break;
            };
            // Reinsert exactly the one wrap-space the intended bytes prove.
            if let Some(remainder) = intended.strip_prefix(candidate.as_str())
                && remainder.starts_with(' ')
            {
                candidate.push(' ');
            }
            candidate.push_str(part);
        }
        return Some(candidate);
    }
    None
}

/// Does the pane show EXACTLY the notice that was staged — `_notice_prove`?
///
/// Sentinels first: the head catches a clipped start, and the trailing
/// `⟧<id>⟧` terminal catches a clipped end, before the full byte compare that
/// decides. Both are cheap; neither is sufficient, which is why the compare
/// still happens.
#[must_use]
pub fn prove(tool: Tool, capture: &str, intended: &str) -> bool {
    let Some(candidate) = reconstruct(tool, capture, intended) else {
        return false;
    };
    let head = head_of(intended);
    if head != GENERIC_HEAD && !head_is_wellformed(&head) {
        return false;
    }
    if head == intended || !candidate.starts_with(&head) {
        return false;
    }
    if let Some(id) = bracketed_id(&head) {
        let terminal = format!("⟧{id}⟧");
        if intended.ends_with(&terminal) && !candidate.ends_with(&terminal) {
            return false;
        }
    }
    candidate == intended
}

/// The head used when the intended text carries no ` LONG BODY` marker at all.
const GENERIC_HEAD: &str = "⟦ae:msg from";

/// The notice's head — everything before ` LONG BODY`, or [`GENERIC_HEAD`].
fn head_of(intended: &str) -> String {
    match intended.find(" LONG BODY") {
        Some(at) if at > 0 => intended[..at].to_owned(),
        _ => GENERIC_HEAD.to_owned(),
    }
}

/// Whether a head is one of the two shapes a notice may carry.
fn head_is_wellformed(head: &str) -> bool {
    head.starts_with("⟦ae:msg from ") || (head.starts_with("⟦ae:task⟧[") && head.ends_with(']'))
}

/// The first `[…]` group in `head`, if any — the request id sentinel.
fn bracketed_id(head: &str) -> Option<&str> {
    let open = head.find('[')?;
    let rest = &head[open + 1..];
    let close = rest.find(']')?;
    (close > 0).then(|| &rest[..close])
}

/// Is this row claude's box rule — blank but for a run of U+2500?
fn is_rule(row: &str) -> bool {
    let trimmed = super::region::trim_posix(row);
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '─')
}

/// Drop CSI styling, keeping every printable byte — including NBSP, which is
/// data for the rejoin.
fn strip_csi(row: &str) -> String {
    let mut out = String::with_capacity(row.len());
    let mut rest = row;
    while let Some(at) = rest.find('\u{1b}') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let Some(body) = after.strip_prefix('[') else {
            // Not a CSI: drop the ESC and carry on, as the frozen `sed` does
            // for anything its pattern misses.
            rest = after;
            continue;
        };
        // `[0-9;?]*[ -/]*[@-~]` — parameters, intermediates, one final byte.
        let mut chars = body.char_indices();
        let mut end = None;
        for (offset, ch) in chars.by_ref() {
            if matches!(ch, '0'..='9' | ';' | '?') {
                continue;
            }
            if (' '..='/').contains(&ch) {
                continue;
            }
            if ('@'..='~').contains(&ch) {
                end = Some(offset + ch.len_utf8());
            }
            break;
        }
        rest = end.map_or(body, |offset| &body[offset..]);
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::region::Tool;
    use super::{LIMIT, Mode, NOTICE_CAP, Refused, compose, prepare, prove, reconstruct};

    fn body(name: &str) -> std::path::PathBuf {
        Path::new("/m/sessions/s/messages").join(name)
    }

    #[test]
    fn a_same_session_pointer_is_session_relative_and_names_the_reply_helper() {
        let notice = compose(
            "ask",
            "ae-1",
            "lead",
            &body("ae-1.ask.abc.txt"),
            9000,
            "s",
            "s",
            Path::new("/m/sessions/s"),
        )
        .expect("a pointer");
        assert_eq!(
            notice,
            "⟦ae:msg from lead⟧[ae-1] LONG BODY 9000 B in your session dir: messages/ae-1.ask.abc.txt — read it first; then run your reply helper: reply ae-1 ⟧ae-1⟧"
        );
    }

    #[test]
    fn a_cross_session_pointer_carries_the_absolute_path_and_the_senders_helper() {
        let notice = compose(
            "review",
            "review-2",
            "lead",
            &body("review-2.review.abc.txt"),
            9000,
            "other",
            "s",
            Path::new("/m/sessions/s"),
        )
        .expect("a pointer");
        assert!(
            notice.contains("/m/sessions/s/messages/review-2.review.abc.txt")
                && notice.contains("reply: /m/sessions/s/reply review-2 ⟧review-2⟧"),
            "`messages/` names a DIFFERENT directory in the recipient's session: {notice}"
        );
    }

    #[test]
    fn the_other_two_arms_are_the_task_head_and_the_refless_default() {
        let task = compose(
            "spawn",
            "t1",
            "ae",
            &body("t1.spawn.abc.txt"),
            9000,
            "s",
            "s",
            Path::new("/m/sessions/s"),
        )
        .expect("a pointer");
        assert!(
            task.starts_with("⟦ae:task⟧[t1] LONG BODY 9000 B") && task.ends_with("then begin ⟧t1⟧"),
            "a spawn is an INSTRUCTION, not transcript chat: {task}"
        );
        let bare = compose(
            "interrupt",
            "",
            "lead",
            &body("msg-x.interrupt.abc.txt"),
            9000,
            "s",
            "s",
            Path::new("/m/sessions/s"),
        )
        .expect("a pointer");
        assert!(
            bare.starts_with("⟦ae:msg from lead⟧[-]") && bare.ends_with("⟧-⟧"),
            "{bare}"
        );
        assert_eq!(
            compose(
                "ask",
                "",
                "lead",
                &body("x.ask.a.txt"),
                9000,
                "s",
                "s",
                Path::new("/m")
            ),
            None,
            "a tracked pointer with no id has no reply contract to name"
        );
        assert_eq!(
            compose(
                "ask",
                "r",
                "lead",
                &body("x.ask.a.log"),
                9000,
                "s",
                "s",
                Path::new("/m")
            ),
            None,
            "the store publishes `.txt`; anything else is not its record"
        );
    }

    #[test]
    fn a_pointer_that_would_not_be_small_is_refused_before_any_paste() {
        let deep: String = std::iter::repeat_n("dir", 90).collect::<Vec<_>>().join("/");
        let long = Path::new("/")
            .join(deep)
            .join("messages")
            .join("r.ask.a.txt");
        assert!(long.display().to_string().len() > NOTICE_CAP);
        assert_eq!(
            compose(
                "ask",
                "r",
                "lead",
                &long,
                9000,
                "other",
                "s",
                Path::new("/m")
            ),
            None,
            "the body stays recoverable where it is; nothing is pasted"
        );
    }

    #[test]
    fn only_an_oversize_body_to_a_modelled_tool_becomes_a_pointer() {
        let file = body("ae-1.ask.abc.txt");
        let call = |tool, bytes| {
            prepare(
                tool,
                "ask",
                "ae-1",
                "lead",
                "s",
                "s",
                &file,
                bytes,
                Path::new("/m/sessions/s"),
            )
        };
        assert_eq!(
            call(Tool::Claude, LIMIT),
            Ok(Mode::Direct),
            "at the limit, not over it"
        );
        assert!(matches!(call(Tool::Claude, LIMIT + 1), Ok(Mode::Notice(_))));
        assert!(matches!(call(Tool::Codex, LIMIT + 1), Ok(Mode::Notice(_))));
        assert_eq!(
            call(Tool::Other, LIMIT + 1),
            Ok(Mode::Direct),
            "an unmodelled TUI has no sensor, so the on-screen proof cannot be taken"
        );
        let unnameable = Path::new("/m/messages/r.ask.a.log");
        assert_eq!(
            prepare(
                Tool::Claude,
                "ask",
                "ae-1",
                "lead",
                "s",
                "s",
                unnameable,
                LIMIT + 1,
                Path::new("/m")
            ),
            Err(Refused)
        );
    }

    /// A claude box, wrapping the pointer across rows with the TWO-SPACE
    /// continuation indent both modelled TUIs draw — and `capture-pane` drops
    /// the visual wrap space at the row boundary, which is the whole reason
    /// the rejoin exists.
    fn wrapped(head: &str, rest: &[&str]) -> String {
        let mut region = String::from("transcript\n\u{1b}[1m❯\u{1b}[0m ");
        region.push_str(head);
        region.push('\n');
        for row in rest {
            region.push_str("  ");
            region.push_str(row);
            region.push('\n');
        }
        region.push_str(&"─".repeat(60));
        region.push_str("\n  model\n");
        region
    }

    #[test]
    fn a_wrapped_pointer_is_rejoined_with_exactly_the_wrap_space_the_bytes_prove() {
        let intended = "⟦ae:msg from lead⟧[ae-1] LONG BODY 9000 B in your session dir: messages/ae-1.ask.abc.txt — read it first ⟧ae-1⟧";
        // The wrap falls INSIDE a word: no space belongs there.
        let inside = wrapped(
            "⟦ae:msg from lead⟧[ae-1] LONG BODY 9000 B in your session dir: messages/ae-1.ask.a",
            &["bc.txt — read it first ⟧ae-1⟧"],
        );
        assert_eq!(
            reconstruct(Tool::Claude, &inside, intended).as_deref(),
            Some(intended)
        );
        assert!(prove(Tool::Claude, &inside, intended));
        // The wrap falls AT a space: exactly one is reinserted.
        let at_space = wrapped(
            "⟦ae:msg from lead⟧[ae-1] LONG BODY 9000 B in your session dir:",
            &["messages/ae-1.ask.abc.txt — read it first ⟧ae-1⟧"],
        );
        assert_eq!(
            reconstruct(Tool::Claude, &at_space, intended).as_deref(),
            Some(intended)
        );
        assert!(prove(Tool::Claude, &at_space, intended));
    }

    #[test]
    fn a_clipped_head_or_tail_is_not_proven() {
        let intended = "⟦ae:msg from lead⟧[ae-1] LONG BODY 9000 B in your session dir: messages/ae-1.ask.abc.txt — read it first ⟧ae-1⟧";
        // The terminal sentinel is missing: the pointer is cut short.
        let clipped = wrapped(
            "⟦ae:msg from lead⟧[ae-1] LONG BODY 9000 B in your session dir: messages/ae-1.ask.a",
            &["bc.txt — read it first"],
        );
        assert!(!prove(Tool::Claude, &clipped, intended));
        // The head is not on any prompt row: a transcript line carrying the
        // same bytes is not the input box.
        let transcript = format!(
            "{intended}\n\u{1b}[1m❯\u{1b}[0m\u{a0}\n{}\n",
            "─".repeat(60)
        );
        assert_eq!(reconstruct(Tool::Claude, &transcript, intended), None);
        assert!(!prove(Tool::Claude, &transcript, intended));
        // An unmodelled tool has no ornament to anchor on.
        assert!(!prove(Tool::Other, &wrapped(intended, &[]), intended));
    }

    #[test]
    fn a_codex_box_ends_at_a_blank_row_where_a_claude_box_ends_at_its_border() {
        let intended = "⟦ae:msg from lead⟧[ae-1] LONG BODY 9000 B: /m/x.ask.a.txt — read it first; reply: /m/reply ae-1 ⟧ae-1⟧";
        let region = "transcript\n\u{1b}[1m›\u{1b}[0m ⟦ae:msg from lead⟧[ae-1] LONG BODY 9000 B: /m/x.ask.a.txt — read it first; reply: /m/reply\n  ae-1 ⟧ae-1⟧\n\n  gpt  ~/x\n";
        assert!(prove(Tool::Codex, region, intended), "{region:?}");
    }
}
