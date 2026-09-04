//! The `requests` read surface.
//!
//! Two things, kept apart the way the frozen implementation keeps them apart:
//! the request SENSOR ([`states`]) and the TABLE the helper prints
//! ([`render`]). The frozen `requests` helper carries `_ar_request_states`
//! beside it by `declare -f` rather than owning a second copy, and the frozen
//! source says why — the two copies had already drifted once, one checking both
//! ends of a reply and the other only the actor end. So the sensor is a public
//! function here, not a private detail of the table.

use std::collections::HashMap;
use std::path::Path;

use crate::event_text::{
    CONTAINER, Member, event_line, extract, member, pad_left_aligned, read_container, read_lines,
    reversed,
};
use crate::events::Identity;
use crate::tmux::ObservedViewer;

/// `requests [mine|inbox|all]` — signature, defaulting to `mine`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Requests this agent opened.
    #[default]
    Mine,
    /// Requests addressed to this agent.
    Inbox,
    /// Every request in the session.
    All,
}

impl Mode {
    /// The mode a token selects, or `None` for a token that is not one.
    ///
    /// `None` for the argument is `mine`: the frozen helper's `${1:-mine}`.
    /// A token that is not one of the three is a USAGE ERROR and is not this
    /// type's to describe — see [`crate::cli::Request`], which carries it to the
    /// crate's one usage exit code.
    ///
    /// ```
    /// use ae::requests::Mode;
    ///
    /// assert_eq!(Mode::parse(None), Some(Mode::Mine));
    /// assert_eq!(Mode::parse(Some("all")), Some(Mode::All));
    /// assert_eq!(Mode::parse(Some("Mine")), None, "the tokens are exact");
    /// ```
    #[must_use]
    pub fn parse(token: Option<&str>) -> Option<Self> {
        match token.unwrap_or("mine") {
            "mine" => Some(Self::Mine),
            "inbox" => Some(Self::Inbox),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    /// Whether this mode needs to know who is asking.
    #[must_use]
    pub fn needs_identity(self) -> bool {
        self != Self::All
    }
}

/// Who is asking — the pane identity the frozen helper reads from tmux.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Viewer {
    /// `ae_current_slot` — the routing key.
    pub slot: String,
    /// The tmux session (`#S`) the calling pane belongs to.
    pub session: String,
    /// `ae_current_agent_ref` — the display `alias:name`.
    pub display: String,
}

impl Viewer {
    /// Whether an identity was detected at all.
    #[must_use]
    pub fn is_known(&self) -> bool {
        !self.display.is_empty()
    }

    /// The viewer a pane's readings make, by the frozen helper's rules.
    #[must_use]
    pub fn from_pane(observed: &ObservedViewer, own_session: &str) -> Self {
        let (Some(agent), Some(session)) = (observed.agent.as_deref(), observed.session.as_deref())
        else {
            return Self::default();
        };
        let display = if session == own_session {
            agent.to_owned()
        } else {
            format!("@{session}:{agent}")
        };
        match observed.slot.as_deref() {
            Some(slot) if is_slot(slot) => Self {
                slot: slot.to_owned(),
                session: session.to_owned(),
                display,
            },
            _ => Self {
                slot: String::new(),
                session: String::new(),
                display,
            },
        }
    }

    /// This viewer, classified by the same rule the rows are.
    fn identity(&self) -> Identity<'_> {
        match (self.slot.is_empty(), self.session.is_empty()) {
            (false, false) => Identity::Routed {
                slot: &self.slot,
                session: &self.session,
            },
            (true, true) => Identity::Display(&self.display),
            _ => Identity::Unassociated,
        }
    }
}

/// One request, as the sensor emits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `pending`, `replied` or `cancelled`.
    pub status: Status,
    /// `ask` or `review`, verbatim from the opening event.
    pub kind: Vec<u8>,
    /// The request id (`ref`).
    pub id: Vec<u8>,
    /// The opening event's `actor`.
    pub from: Vec<u8>,
    /// The opening event's `target`.
    pub to: Vec<u8>,
    /// The opening event's `ts`.
    pub at: Vec<u8>,
    /// The opening event's `body_file`.
    pub body_file: Vec<u8>,
    /// Routing key of the sender.
    pub from_slot: Key,
    /// Routing key of the target.
    pub to_slot: Key,
    /// Session of the sender's routing key.
    pub from_session: Key,
    /// Session of the target's routing key.
    pub to_session: Key,
    /// The DISPLAY summary: the request's own text while pending, the closing
    /// event's text once closed.
    pub summary: Vec<u8>,
}

/// The three terminal states of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No valid closing event.
    Pending,
    /// Closed by a full-mirror reply.
    Replied,
    /// Withdrawn by its own sender. Terminal against a later reply.
    Cancelled,
}

impl Status {
    /// The token the table prints.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Replied => "replied",
            Self::Cancelled => "cancelled",
        }
    }
}

impl Request {
    /// Whether `mode` shows this row to `viewer`.
    #[must_use]
    pub fn shown_to(&self, mode: Mode, viewer: &Viewer) -> bool {
        match mode {
            Mode::All => true,
            Mode::Mine => self.asker_identity().matches(viewer.identity()),
            Mode::Inbox => self.askee_identity().matches(viewer.identity()),
        }
    }

    /// This row's sender, as an identity.
    #[must_use]
    fn asker_identity(&self) -> Identity<'_> {
        identity_of(&self.from_slot, &self.from_session, &self.from)
    }

    /// This row's target, as an identity.
    #[must_use]
    fn askee_identity(&self) -> Identity<'_> {
        identity_of(&self.to_slot, &self.to_session, &self.to)
    }

    /// The table line for this row, `\n` included.
    fn write_line(&self, out: &mut Vec<u8>) {
        write_row(
            out,
            self.status.token().as_bytes(),
            &self.kind,
            &self.id,
            &self.from,
            &self.to,
            &self.summary,
        );
    }
}

/// `printf "%-8s %-8s %-28s %-20s %-20s %s\n"` — the one format, used for the
/// header and every row, so the two can never drift apart.
fn write_row(
    out: &mut Vec<u8>,
    status: &[u8],
    kind: &[u8],
    id: &[u8],
    from: &[u8],
    to: &[u8],
    summary: &[u8],
) {
    for (field, width) in [(status, 8), (kind, 8), (id, 28), (from, 20), (to, 20)] {
        pad_left_aligned(out, field, width);
        out.push(b' ');
    }
    out.extend_from_slice(summary);
    out.push(b'\n');
}

/// Whether `text` is a routing slot: exactly `main`, `worker.<digits>` or
/// `spawned.<digits>` — the frozen `_valid_slot`, anchored, so a tampered
/// `@ae_slot` such as `worker.0x` cannot route.
#[must_use]
pub fn is_slot(text: &str) -> bool {
    if text == "main" {
        return true;
    }
    ["worker.", "spawned."].iter().any(|prefix| {
        text.strip_prefix(prefix)
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// The refusal for `mine`/`inbox` when no identity could be detected.
pub const NO_IDENTITY: &str =
    "Error: could not detect current agent identity; use 'requests all' outside an ae pane";

/// The exit status of the identity refusal.
pub const EXIT_NO_IDENTITY: u8 = 1;

/// The header line, byte for byte.
#[must_use]
pub fn header() -> Vec<u8> {
    let mut out = Vec::new();
    write_row(
        &mut out, b"STATUS", b"TYPE", b"ID", b"FROM", b"TO", b"SUMMARY",
    );
    out
}

/// What the surface writes, and the status it exits with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The table. Empty on a refusal — the frozen helper refuses BEFORE the
    /// header, so a refused invocation writes nothing to stdout at all.
    pub stdout: Vec<u8>,
    /// The refusal. Empty on success.
    pub stderr: Vec<u8>,
    /// `0` for a table, [`EXIT_NO_IDENTITY`] for the refusal.
    pub code: u8,
}

/// Render the surface over the session meta directory `dir`.
#[must_use]
pub fn render(dir: &Path, mode: Mode, viewer: &Viewer) -> Output {
    if mode.needs_identity() && !viewer.is_known() {
        return Output {
            stdout: Vec::new(),
            stderr: format!("{NO_IDENTITY}\n").into_bytes(),
            code: EXIT_NO_IDENTITY,
        };
    }
    let container = read_container(&dir.join(CONTAINER));
    Output {
        stdout: table(&container, mode, viewer),
        stderr: Vec::new(),
        code: 0,
    }
}

/// The table for one container's bytes — the pure half of [`render`].
#[must_use]
pub fn table(container: &[u8], mode: Mode, viewer: &Viewer) -> Vec<u8> {
    let mut out = header();
    for request in states(container) {
        if request.shown_to(mode, viewer) {
            request.write_line(&mut out);
        }
    }
    out
}

/// THE request sensor — one definition, chronological, one row per request.
#[must_use]
pub fn states(container: &[u8]) -> Vec<Request> {
    let stream = reversed(container);
    // Every retained record carries its SCAN ORDINAL — its position in the
    // container, i.e. LEDGER ORDER, which is APPEND ORDER and NEVER `ts`
    // (colead ruling, 2026-08-24). `ts` is read and published as a field and is
    let mut opened: HashMap<Vec<u8>, (usize, Opening)> = HashMap::new();
    let mut replies: HashMap<Vec<u8>, Vec<(usize, Closing)>> = HashMap::new();
    let mut cancels: HashMap<Vec<u8>, Vec<(usize, Closing)>> = HashMap::new();
    // Newest first, because that is the order they are met in.
    let mut refs: Vec<Vec<u8>> = Vec::new();
    let mut scan = 0_usize;

    for line in read_lines(&stream) {
        let Some(line) = event_line(line) else {
            continue;
        };
        // Counted for every brace-prefixed line, before the `ref` test, exactly
        // where the frozen sensor counts it.
        scan += 1;
        let reference = extract(line, "ref");
        if reference.is_empty() {
            continue;
        }
        match extract(line, "action").as_slice() {
            action @ (b"ask" | b"review") => {
                // First seen in a newest-first scan is the NEWEST opening.
                if opened.contains_key(&reference) {
                    continue;
                }
                opened.insert(reference.clone(), (scan, Opening::read(line, action)));
                refs.push(reference);
            }
            b"reply" => replies
                .entry(reference)
                .or_default()
                .push((scan, Closing::read(line))),
            b"cancel" => cancels
                .entry(reference)
                .or_default()
                .push((scan, Closing::read(line))),
            _ => {}
        }
    }

    // Reversing the newest-first encounter order gives chronological output.
    refs.reverse();
    refs.into_iter()
        .filter_map(|reference| {
            let (opened_at, opening) = opened.remove(&reference)?;
            let no_candidates = Vec::new();
            // a terminal event terminates only the newest opening
            // that PRECEDES it. The row shown for a ref is its newest opening,
            // so a terminal attaches to it exactly when the terminal came
            let after = |(at, _): &&(usize, Closing)| *at < opened_at;
            let cancelled = cancels.get(&reference).unwrap_or(&no_candidates);
            let answered = replies.get(&reference).unwrap_or(&no_candidates);
            // Candidates were appended newest-first, so the FIRST that passes
            // both tests is the newest that counts — and an invalid newer one
            // cannot bury a valid older one, because validity is decided here
            let cancel = cancelled
                .iter()
                .filter(after)
                .find(|(_, candidate)| opening.withdrawn_by(candidate));
            let reply = answered
                .iter()
                .filter(after)
                .find(|(_, candidate)| opening.answered_by(candidate));
            // A valid withdrawal wins over any reply, however late: a straggler
            // answer must not reopen a request nobody is waiting on.
            let (status, summary) = match (cancel, reply) {
                (Some((_, closing)), _) => (Status::Cancelled, closing.summary.clone()),
                (None, Some((_, closing))) => (Status::Replied, closing.summary.clone()),
                (None, None) => (Status::Pending, opening.summary.clone()),
            };
            Some(Request {
                status,
                kind: opening.kind,
                id: reference,
                from: opening.from,
                to: opening.to,
                at: opening.at,
                body_file: opening.body_file,
                from_slot: opening.from_slot,
                to_slot: opening.to_slot,
                from_session: opening.from_session,
                to_session: opening.to_session,
                summary,
            })
        })
        .collect()
}

/// One routing member, owned — [`Member`]'s three states, kept past the life of
/// the line they were read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// The key is not in the record.
    Absent,
    /// The key is present and resolves to nothing.
    Empty,
    /// The key carries a value.
    Value(Vec<u8>),
}

impl Key {
    /// The value, or `None` when the key is absent; `Empty` answers `Some(&[])`.
    #[must_use]
    pub fn value(&self) -> Option<&[u8]> {
        match self {
            Self::Absent => None,
            Self::Empty => Some(&[]),
            Self::Value(bytes) => Some(bytes),
        }
    }

    /// Read one routing member off an event line.
    fn read(line: &[u8], name: &str) -> Self {
        match member(line, name) {
            Member::Absent => Self::Absent,
            Member::Empty => Self::Empty,
            Member::Value(bytes) => Self::Value(bytes.into_owned()),
        }
    }
}

/// This side's participant, as [`Identity`] names one — or `Unassociated` when
/// the bytes cannot be one.
fn identity_of<'a>(slot: &'a Key, session: &'a Key, display: &'a [u8]) -> Identity<'a> {
    let text = |bytes: &'a [u8]| std::str::from_utf8(bytes).ok();
    match (slot, session) {
        (Key::Value(slot), Key::Value(session)) => match (text(slot), text(session)) {
            (Some(slot), Some(session)) => Identity::Routed { slot, session },
            _ => Identity::Unassociated,
        },
        (Key::Absent, Key::Absent) => match text(display) {
            Some(display) => Identity::Display(display),
            None => Identity::Unassociated,
        },
        _ => Identity::Unassociated,
    }
}

/// The `ask`/`review` that opened a request.
struct Opening {
    kind: Vec<u8>,
    from: Vec<u8>,
    to: Vec<u8>,
    at: Vec<u8>,
    body_file: Vec<u8>,
    /// Kept as KEYS rather than bytes: the ruled identity rule needs absent and
    /// present-but-empty told apart, and bytes cannot.
    from_slot: Key,
    to_slot: Key,
    from_session: Key,
    to_session: Key,
    summary: Vec<u8>,
}

/// A candidate `reply` or `cancel`. One shape for both: a cancel simply has no
/// target side, and reading the fields it does not carry as empty is what the
/// frozen sensor does too.
struct Closing {
    actor: Vec<u8>,
    target: Vec<u8>,
    actor_slot: Key,
    target_slot: Key,
    actor_session: Key,
    target_session: Key,
    summary: Vec<u8>,
}

impl Opening {
    fn read(line: &[u8], action: &[u8]) -> Self {
        Self {
            kind: action.to_vec(),
            from: extract(line, "actor"),
            to: extract(line, "target"),
            at: extract(line, "ts"),
            body_file: extract(line, "body_file"),
            from_slot: Key::read(line, "actor_slot"),
            to_slot: Key::read(line, "target_slot"),
            from_session: Key::read(line, "actor_session"),
            to_session: Key::read(line, "target_session"),
            summary: fold_newlines(extract(line, "summary")),
        }
    }

    /// This request's SENDER.
    fn asker(&self) -> Identity<'_> {
        identity_of(&self.from_slot, &self.from_session, &self.from)
    }

    /// This request's TARGET.
    fn askee(&self) -> Identity<'_> {
        identity_of(&self.to_slot, &self.to_session, &self.to)
    }

    /// **Strict** — the full mirror: the reply's actor is this
    /// request's TARGET and the reply's target is this request's SENDER, with
    /// both comparisons made by [`Identity::matches`]. A mixed pair closes
    /// nothing.
    fn answered_by(&self, reply: &Closing) -> bool {
        self.askee().matches(reply.actor_identity())
            && self.asker().matches(reply.target_identity())
    }

    /// **RULED (operational lead, 2026-08-27)** — see the module docs. A cancel
    /// withdraws this request when its actor identity is this request's SENDER
    /// by the same strict [`Identity::matches`] the reply mirror uses, or — the
    /// one arm that rule cannot express — when the cancel is SLOTLESS (no
    /// routing member at all, hence a display identity), this request's
    /// opening is ROUTED or is a SLOTLESS-SENDER opening (actor session
    /// present, actor slot ABSENT — the shape both writers give an ask made
    /// under `AE_SENDER_OVERRIDE` from a pane), and the cancel's non-empty
    /// actor bytes equal this request's actor bytes. Nothing looser: a cancel
    /// carrying slot data, right or wrong, is judged by slot + session alone;
    /// an empty actor names nobody; an opening whose slot member is PRESENT
    /// but empty is a writer bug, not that shape, and stays pending. The arm
    /// is for a request asked from a pane under an explicit override and
    /// withdrawn from a pane-less command under the same override —
    /// `ae compact --digest-only` — where the override's bytes are the only
    /// identity the two events share.
    fn withdrawn_by(&self, cancel: &Closing) -> bool {
        let asker = self.asker();
        let slotless_sender =
            matches!(self.from_slot, Key::Absent) && !matches!(self.from_session, Key::Absent);
        // "No routing member at all" means all FOUR: a Display actor identity
        // only says the actor pair is absent, and a cancel carrying a target
        // slot or session is carrying routing data.
        let slotless_cancel = matches!(cancel.actor_slot, Key::Absent)
            && matches!(cancel.actor_session, Key::Absent)
            && matches!(cancel.target_slot, Key::Absent)
            && matches!(cancel.target_session, Key::Absent);
        match cancel.actor_identity() {
            Identity::Display(actor)
                if slotless_cancel
                    && (matches!(asker, Identity::Routed { .. }) || slotless_sender) =>
            {
                !actor.is_empty() && self.from == actor.as_bytes()
            }
            identity => asker.matches(identity),
        }
    }
}

impl Closing {
    fn actor_identity(&self) -> Identity<'_> {
        identity_of(&self.actor_slot, &self.actor_session, &self.actor)
    }

    fn target_identity(&self) -> Identity<'_> {
        identity_of(&self.target_slot, &self.target_session, &self.target)
    }

    fn read(line: &[u8]) -> Self {
        Self {
            actor: extract(line, "actor"),
            target: extract(line, "target"),
            actor_slot: Key::read(line, "actor_slot"),
            target_slot: Key::read(line, "target_slot"),
            actor_session: Key::read(line, "actor_session"),
            target_session: Key::read(line, "target_session"),
            summary: fold_newlines(extract(line, "summary")),
        }
    }
}

/// The frozen sensor's `${_EV//$'\n'/ }` on every summary it stores.
fn fold_newlines(mut value: Vec<u8>) -> Vec<u8> {
    for byte in &mut value {
        if *byte == b'\n' {
            *byte = b' ';
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{
        EXIT_NO_IDENTITY, Key, Mode, NO_IDENTITY, Status, Viewer, header, is_slot, render, states,
        table,
    };
    use std::fs;
    use std::path::PathBuf;

    /// One session's events, as a `\n`-joined container.
    fn container(lines: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for line in lines {
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
        }
        out
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("test fixtures are utf-8")
    }

    const ASK: &str = r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","summary":"the question"}"#;
    /// A ROUTED opening whose actor is a reserved name — the shape the ruling
    /// named literally.
    const ROUTED_ASK: &str = r#"{"ts":"t1","actor":"ae:compact:u1","action":"ask","target":"a:lead","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"main","target_session":"s","summary":"handover"}"#;
    /// `ae compact`'s handover ask as BOTH writers emit it from a pane (live
    /// capture, 2026-08-27): the reserved override as the actor, the pane's
    /// session, and NO actor slot — "external senders have none".
    const HALFKEY_ASK: &str = r#"{"ts":"t1","actor":"ae:compact:u1","action":"ask","target":"a:lead","ref":"r1","actor_session":"s","target_slot":"main","target_session":"s","summary":"handover"}"#;
    /// Not a shape any writer emits: an EMPTY actor (the emitter falls back to
    /// `human`). A corrupt row, kept so the arm's non-empty guard is proven
    /// rather than assumed.
    const EMPTYACTOR_ASK: &str = r#"{"ts":"t1","actor":"","action":"ask","target":"a:lead","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"main","target_session":"s","summary":"corrupt"}"#;
    /// Not the frozen shape: a PRESENT-but-empty actor slot is a writer bug,
    /// and stays half a key.
    const EMPTYSLOT_ASK: &str = r#"{"ts":"t1","actor":"ae:compact:u1","action":"ask","target":"a:lead","ref":"r1","actor_slot":"","actor_session":"s","target_slot":"main","target_session":"s","summary":"handover"}"#;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("ae-requests-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_header_is_the_frozen_bytes() {
        // Transcribed from the frozen capture
        // arms/A1/c01-healthy-ro/out/requests-all.stdout, whose first line this
        // is. Spelled out rather than built from the same widths the code uses,
        assert_eq!(
            text(&header()),
            "STATUS   TYPE     ID                           FROM                 TO                   SUMMARY\n"
        );
    }

    #[test]
    fn an_empty_container_is_the_header_alone() {
        assert_eq!(table(b"", Mode::All, &Viewer::default()), header());
    }

    #[test]
    fn a_missing_container_is_quiet_and_succeeds() {
        let scratch = Scratch::new("absent");
        let out = render(&scratch.0, Mode::All, &Viewer::default());
        assert_eq!(out.stdout, header());
        assert!(out.stderr.is_empty(), "absent is quiet, not degraded");
        assert_eq!(out.code, 0);
    }

    #[test]
    fn mine_and_inbox_refuse_before_the_header_when_no_identity_was_detected() {
        let scratch = Scratch::new("noid");
        fs::write(scratch.0.join("events.jsonl"), container(&[ASK])).expect("write");
        for mode in [Mode::Mine, Mode::Inbox] {
            let out = render(&scratch.0, mode, &Viewer::default());
            assert!(
                out.stdout.is_empty(),
                "{mode:?}: the refusal precedes the header"
            );
            assert_eq!(text(&out.stderr), format!("{NO_IDENTITY}\n"), "{mode:?}");
            assert_eq!(out.code, EXIT_NO_IDENTITY, "{mode:?}");
            assert_ne!(out.code, 2, "{mode:?}: pinned to 1, not the usage code");
        }
    }

    #[test]
    fn all_needs_no_identity() {
        let scratch = Scratch::new("all-noid");
        fs::write(scratch.0.join("events.jsonl"), container(&[ASK])).expect("write");
        let out = render(&scratch.0, Mode::All, &Viewer::default());
        assert_eq!(out.code, 0);
        assert!(text(&out.stdout).contains("the question"));
    }

    #[test]
    fn sc_518_a_full_mirror_reply_closes_and_carries_its_own_text() {
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"the answer"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Replied);
        assert_eq!(rows[0].summary, b"the answer", "the DISPLAY summary closes");
    }

    #[test]
    fn sc_518_a_reply_addressed_elsewhere_leaves_the_request_pending() {
        // Both halves of the mirror, one at a time. This is the exact drift the
        // frozen source records: a sensor that checked only the actor end.
        let wrong_target = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:third","ref":"r1","summary":"misaddressed"}"#,
        ]);
        let wrong_actor = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:third","action":"reply","target":"a:lead","ref":"r1","summary":"stranger"}"#,
        ]);
        for (body, why) in [(wrong_target, "target end"), (wrong_actor, "actor end")] {
            let rows = states(&body);
            assert_eq!(rows[0].status, Status::Pending, "{why}");
            assert_eq!(rows[0].summary, b"the question", "{why}");
        }
    }

    #[test]
    fn sc_518_routing_keys_decide_when_both_sides_carry_them() {
        // Same display names on both sides, opposed routing keys: the mirror
        // must fail on the keys, which name-matching alone would pass.
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"q"}"#,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"spawned.3","actor_session":"s","target_slot":"main","target_session":"s","summary":"wrong slot"}"#,
        ]);
        assert_eq!(states(&body)[0].status, Status::Pending);

        let session_mismatch = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"q"}"#,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"worker.0","actor_session":"other","target_slot":"main","target_session":"s","summary":"wrong session"}"#,
        ]);
        assert_eq!(states(&session_mismatch)[0].status, Status::Pending);
    }

    #[test]
    fn sc_518_a_keyless_request_is_not_closed_by_a_routed_reply() {
        // GAP 1, THE ASYMMETRIC DIRECTION — the shape the corpus does not have.
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"worker.0","actor_session":"s","target_slot":"main","target_session":"s","summary":"routed reply"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows[0].status, Status::Pending);
        assert_eq!(
            rows[0].summary, b"the question",
            "and it keeps the opening's own text"
        );

        // The control that makes the assertion mean something: the SAME opening
        // closed by a keyless reply, which is Display to Display and DOES
        // close. Without this arm the test would also pass on an
        let both_keyless = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"keyless reply"}"#,
        ]);
        assert_eq!(states(&both_keyless)[0].status, Status::Replied);
    }

    #[test]
    fn sc_518_an_empty_routing_member_is_not_an_absent_one() {
        // GAP 5 (lexec's): the ONE shape where Empty and Absent diverge, and the
        // one a reader who thinks they are the same gets wrong while passing
        // every byte the corpus owns.
        let keyless_reply = r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"absent keys"}"#;
        let empty_reply = r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"","actor_session":"","target_slot":"","target_session":"","summary":"empty keys"}"#;

        // ASK is keyless, so the opening is a Display identity.
        assert_eq!(
            states(&container(&[ASK, keyless_reply]))[0].status,
            Status::Replied,
            "absent on both sides is the display fallback"
        );
        assert_eq!(
            states(&container(&[ASK, empty_reply]))[0].status,
            Status::Pending,
            "present-and-empty is Unassociated, and matches nothing"
        );

        // And the pair that shows why the corpus could not tell them apart: a
        // routed opening refuses both, so every captured shape agrees.
        let routed_ask = r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"q"}"#;
        for reply in [keyless_reply, empty_reply] {
            assert_eq!(
                states(&container(&[routed_ask, reply]))[0].status,
                Status::Pending,
                "a routed opening refuses both, which is why no capture separates them"
            );
        }
    }

    #[test]
    fn sc_518_half_a_routing_key_names_nobody_even_beside_a_display_opening() {
        // THE THIRD SHAPE IN THE Unassociated FAMILY, and the corpus cannot see
        // it either. `Empty` and `Absent` have a sibling: exactly ONE routing
        // member present. Against a ROUTED opening it fails like everything
        let half_keyed_reply = r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"worker.0","target_slot":"main","summary":"half a key"}"#;
        // ASK is keyless, so its participants are Display identities.
        assert_eq!(
            states(&container(&[ASK, half_keyed_reply]))[0].status,
            Status::Pending,
            "a slot with no session names nobody, and does not become its display name"
        );
        // The control, on the same opening: a fully keyless reply DOES close it,
        // so the assertion above is about the half key and not about the reply.
        let keyless_reply = r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"no keys"}"#;
        assert_eq!(
            states(&container(&[ASK, keyless_reply]))[0].status,
            Status::Replied
        );
        // And the other half of the pair, for completeness: a session with no
        // slot is the same species.
        let session_only_reply = r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_session":"s","target_session":"s","summary":"the other half"}"#;
        assert_eq!(
            states(&container(&[ASK, session_only_reply]))[0].status,
            Status::Pending
        );
    }

    #[test]
    fn an_identity_that_is_not_valid_utf8_names_nobody() {
        // THE UTF-8 GATE, as a test rather than as a paragraph. `Identity`
        // compares `&str` and this surface reads arbitrary bytes, so the
        // conversion needs a decision for the impossible case — and the
        let mut ask = Vec::from(
            &br#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":""#[..],
        );
        ask.extend_from_slice(&[0xFF, 0xFE]);
        ask.extend_from_slice(br#"","target_session":"s","summary":"q"}"#);
        let mut reply = Vec::from(
            &br#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":""#[..],
        );
        reply.extend_from_slice(&[0xFF, 0xFE]);
        reply.extend_from_slice(
            br#"","actor_session":"s","target_slot":"main","target_session":"s","summary":"a"}"#,
        );
        let mut body = ask;
        body.push(b'\n');
        body.extend_from_slice(&reply);
        body.push(b'\n');

        let rows = states(&body);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].status,
            Status::Pending,
            "identical undecodable slots are still not an identity"
        );
        // The member was READ — this is a gate on the comparison, not on the
        // extraction, and the published row still carries the bytes verbatim.
        assert_eq!(rows[0].to_slot, Key::Value(vec![0xFF, 0xFE]));
    }

    #[test]
    fn sc_518a_a_terminal_that_precedes_its_opening_closes_nothing() {
        // The ORDER rule alone, with identity deliberately PERFECT so a failure
        // can only be about ordering. This is the corpus's one specimen shape
        // (G5/m2) reduced to its mechanism.
        let reply_first = container(&[
            r#"{"ts":"t1","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"answered before it was asked"}"#,
            ASK,
        ]);
        let rows = states(&reply_first);
        assert_eq!(rows[0].status, Status::Pending);
        assert_eq!(rows[0].summary, b"the question");

        // The control, and it is what makes the assertion about ORDER: the same
        // two records the other way round DO close. Without this arm the test
        // would pass on an implementation whose identity rule rejected the
        let ask_first = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"answered after it was asked"}"#,
        ]);
        assert_eq!(states(&ask_first)[0].status, Status::Replied);
    }

    #[test]
    fn sc_518a_a_re_ask_opens_a_new_lifecycle_that_the_old_terminal_cannot_close() {
        // GAP 2. Identity is perfect throughout, so this test can ONLY fail on
        // the ordering rule — which is the point: a re-ask whose reply is also
        // identity-invalid would be pending for two reasons and could not tell
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"the first answer"}"#,
            r#"{"ts":"t3","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","summary":"asked again"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows.len(), 1, "one row per ref");
        assert_eq!(
            rows[0].status,
            Status::Pending,
            "the new lifecycle is not born closed by the old lifecycle's reply"
        );
        assert_eq!(rows[0].summary, b"asked again");

        // And the earlier terminal is not merely ignored — a reply AFTER the
        // re-ask closes the new lifecycle, so the rule is about reach and not
        // about discarding replies.
        let answered_again = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"the first answer"}"#,
            r#"{"ts":"t3","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","summary":"asked again"}"#,
            r#"{"ts":"t4","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"the second answer"}"#,
        ]);
        let rows = states(&answered_again);
        assert_eq!(rows[0].status, Status::Replied);
        assert_eq!(rows[0].summary, b"the second answer");
    }

    #[test]
    fn sc_518a_a_cancel_before_its_opening_has_no_effect_whatever_authorization_says() {
        // GAP 3, AND OUTCOME-NEUTRAL BY CONSTRUCTION.
        let re_ask = r#"{"ts":"t3","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","summary":"asked again"}"#;
        let own =
            r#"{"ts":"t2","actor":"a:lead","action":"cancel","ref":"r1","summary":"withdrawn"}"#;
        let stranger =
            r#"{"ts":"t2","actor":"a:stranger","action":"cancel","ref":"r1","summary":"not mine"}"#;
        let anonymous = r#"{"ts":"t2","action":"cancel","ref":"r1","summary":"nobody"}"#;

        let without = states(&container(&[ASK]));
        for cancel in [own, stranger, anonymous] {
            // The cancel precedes the opening. Whoever sent it, it is a no-op.
            let with = states(&container(&[cancel, ASK]));
            assert_eq!(
                with, without,
                "a pre-opening cancel changed the row: {cancel}"
            );
        }

        // The same rule from the re-ask side: an earlier lifecycle's cancel
        // cannot reach forward, so the re-asked row is identical to the row of a
        // container holding only the re-ask. Also neutral — it says nothing
        let only_re_ask = states(&container(&[re_ask]));
        for cancel in [own, stranger, anonymous] {
            let re_asked = states(&container(&[ASK, cancel, re_ask]));
            assert_eq!(
                re_asked, only_re_ask,
                "an earlier lifecycle's cancel reached the re-ask: {cancel}"
            );
        }
    }

    /// **NON-GATING DIAGNOSTIC** — a `cancel` AND a `reply` both attached to one
    /// opening. GAP 4, and the gate now says NOTHING about it.
    #[test]
    #[ignore = "records unratified two-terminal precedence behavior; not a gate"]
    fn diagnostic_two_terminals_on_one_opening() {
        let cancel_then_reply = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:lead","action":"cancel","ref":"r1","summary":"never mind"}"#,
            r#"{"ts":"t3","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"too late"}"#,
        ]);
        let reply_then_cancel = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"answered"}"#,
            r#"{"ts":"t3","actor":"a:lead","action":"cancel","ref":"r1","summary":"withdrawn anyway"}"#,
        ]);
        // IS, not contract, and BOTH orders are recorded rather than related:
        // this build resolves by KIND (cancellation wins) rather than by
        // recency, so the two agree — but that agreement is a property of the
        assert_eq!(states(&cancel_then_reply)[0].status, Status::Cancelled);
        assert_eq!(states(&reply_then_cancel)[0].status, Status::Cancelled);
    }

    /// The ruling's strict half, gated: the request's own sender withdraws it —
    /// a stranger and an unattributed cancel do not. Formerly the `#[ignore]`d
    /// diagnostic of the interim policy; the ruling kept that half as it was,
    /// so the record of what had to change here is: nothing.
    #[test]
    fn the_request_s_own_sender_withdraws_it_and_nobody_else_does() {
        let after = |actor: &str, summary: &str| {
            let cancel = format!(
                r#"{{"ts":"t2",{actor}"action":"cancel","ref":"r1","summary":"{summary}"}}"#
            );
            states(&container(&[ASK, &cancel]))[0].status
        };
        assert_eq!(
            after(r#""actor":"a:lead","#, "withdrawn"),
            Status::Cancelled
        );
        assert_eq!(
            after(r#""actor":"a:stranger","#, "not mine"),
            Status::Pending
        );
        assert_eq!(after("", "nobody"), Status::Pending);
    }

    #[test]
    fn an_invalid_newer_reply_cannot_bury_a_valid_older_one() {
        // The measured regression the frozen sensor's retain-then-validate shape
        // exists to prevent: keeping only the newest RAW candidate and
        // validating it afterwards rendered this `pending`.
        let replies = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"valid answer"}"#,
            r#"{"ts":"t3","actor":"a:stranger","action":"reply","target":"a:lead","ref":"r1","summary":"stranger answer"}"#,
        ]);
        let rows = states(&replies);
        assert_eq!(rows[0].status, Status::Replied);
        assert_eq!(rows[0].summary, b"valid answer");
    }

    /// The cancel half of retain-then-validate, gated now that cancel
    /// authorization is ruled: an invalid newer cancel cannot bury a valid
    /// older one.
    #[test]
    fn an_invalid_newer_cancel_cannot_bury_a_valid_older_one() {
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:lead","action":"cancel","ref":"r1","summary":"valid withdrawal"}"#,
            r#"{"ts":"t3","actor":"a:stranger","action":"cancel","ref":"r1","summary":"stranger cancel"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows[0].status, Status::Cancelled);
        assert_eq!(rows[0].summary, b"valid withdrawal");
    }

    /// The ruled arm, gated: a SLOTLESS cancel withdraws a ROUTED or a
    /// SLOTLESS-SENDER request on exact, non-empty actor bytes — `ae compact
    /// --digest-only`'s shape — and on nothing looser. An opening whose slot
    /// member is present but empty is neither, and stays pending.
    #[test]
    fn a_slotless_cancel_withdraws_on_exact_actor_bytes_only() {
        let after = |opening: &str, actor_member: &str| {
            let cancel = format!(
                r#"{{"ts":"t2",{actor_member}"action":"cancel","ref":"r1","summary":"withdrawn"}}"#
            );
            states(&container(&[opening, &cancel]))[0].status
        };
        for opening in [ROUTED_ASK, HALFKEY_ASK] {
            assert_eq!(
                after(opening, r#""actor":"ae:compact:u1","#),
                Status::Cancelled,
                "exact bytes, no slot data: {opening}"
            );
            assert_eq!(
                after(opening, r#""actor":"ae:compact:u2","#),
                Status::Pending,
                "another actor"
            );
            assert_eq!(
                after(opening, r#""actor":"AE:COMPACT:U1","#),
                Status::Pending,
                "bytes, not a case-fold"
            );
            assert_eq!(
                after(opening, r#""actor":"ae:compact:u1 ","#),
                Status::Pending,
                "bytes, not a trim"
            );
            assert_eq!(
                after(opening, r#""actor":"","#),
                Status::Pending,
                "an empty actor names nobody"
            );
            assert_eq!(after(opening, ""), Status::Pending, "no actor names nobody");
        }
        assert_eq!(
            after(EMPTYSLOT_ASK, r#""actor":"ae:compact:u1","#),
            Status::Pending,
            "a present-but-empty opening slot is half a key, not the slotless-sender shape"
        );
        // Equal bytes alone would close this: the guard is what keeps two
        // empty actors from naming the same nobody.
        assert_eq!(
            after(EMPTYACTOR_ASK, r#""actor":"","#),
            Status::Pending,
            "empty equals empty is not an identity"
        );
    }

    /// Slot data on the cancel, right or wrong, is judged by slot + session
    /// alone, exactly as a reply's is: equal actor bytes do not rescue a
    /// mismatched key, a matching key does not need them, and half a key
    /// names nobody even beside equal bytes.
    #[test]
    fn a_stamped_cancel_stays_under_strict_slot_and_session_matching() {
        let after = |members: &str| {
            let cancel = format!(
                r#"{{"ts":"t2","action":"cancel","ref":"r1",{members}"summary":"withdrawn"}}"#
            );
            states(&container(&[ROUTED_ASK, &cancel]))[0].status
        };
        assert_eq!(
            after(r#""actor":"ae:compact:u1","actor_slot":"worker.0","actor_session":"s","#),
            Status::Pending,
            "same bytes, another slot"
        );
        assert_eq!(
            after(r#""actor":"ae:compact:u1","actor_slot":"main","actor_session":"other","#),
            Status::Pending,
            "same bytes, another session"
        );
        assert_eq!(
            after(r#""actor":"renamed:lead","actor_slot":"main","actor_session":"s","#),
            Status::Cancelled,
            "the key matches; the display need not"
        );
        assert_eq!(
            after(r#""actor":"ae:compact:u1","actor_slot":"","actor_session":"","#),
            Status::Pending,
            "present-and-empty is half a key"
        );
        assert_eq!(
            after(r#""actor":"ae:compact:u1","actor_slot":"main","#),
            Status::Pending,
            "one member of two is half a key"
        );
    }

    /// "No routing member at all" is all four: a cancel whose ACTOR side is
    /// bare but which carries a target slot or session — a value or a
    /// present-but-empty one — is carrying routing data, and stays under the
    /// strict rule on both opening shapes.
    #[test]
    fn a_cancel_carrying_a_target_member_is_not_slotless() {
        let after = |opening: &str, members: &str| {
            let cancel = format!(
                r#"{{"ts":"t2","actor":"ae:compact:u1","action":"cancel","ref":"r1",{members}"summary":"withdrawn"}}"#
            );
            states(&container(&[opening, &cancel]))[0].status
        };
        for opening in [ROUTED_ASK, HALFKEY_ASK] {
            assert_eq!(
                after(opening, ""),
                Status::Cancelled,
                "the bare shape: {opening}"
            );
            for members in [
                r#""target_slot":"main","#,
                r#""target_session":"s","#,
                r#""target_slot":"main","target_session":"s","#,
                r#""target_slot":"","#,
                r#""target_session":"","#,
                r#""target_slot":"","target_session":"","#,
            ] {
                assert_eq!(
                    after(opening, members),
                    Status::Pending,
                    "a target member is routing data: {members} on {opening}"
                );
            }
        }
    }

    /// order rule under the ruled arm, in the positive: it withdraws
    /// only the opening it FOLLOWS, and a re-ask opens a lifecycle an earlier
    /// withdrawal cannot reach.
    #[test]
    fn the_slotless_arm_obeys_causality_and_a_re_ask() {
        let own = r#"{"ts":"t2","actor":"ae:compact:u1","action":"cancel","ref":"r1","summary":"withdrawn"}"#;
        let re_ask = r#"{"ts":"t3","actor":"ae:compact:u1","action":"ask","target":"a:lead","ref":"r1","actor_session":"s","target_slot":"main","target_session":"s","summary":"asked again"}"#;
        for opening in [ROUTED_ASK, HALFKEY_ASK] {
            assert_eq!(
                states(&container(&[opening, own]))[0].status,
                Status::Cancelled
            );
            assert_eq!(
                states(&container(&[own, opening])),
                states(&container(&[opening])),
                "before its opening: no effect"
            );
            let re_asked = states(&container(&[opening, own, re_ask]));
            assert_eq!(
                re_asked,
                states(&container(&[re_ask])),
                "an earlier lifecycle's withdrawal cannot reach the re-ask"
            );
            assert_eq!(re_asked[0].status, Status::Pending);
        }
    }

    #[test]
    fn the_newest_valid_candidate_of_a_kind_is_the_one_that_counts() {
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"older answer"}"#,
            r#"{"ts":"t3","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"newer answer"}"#,
        ]);
        assert_eq!(states(&body)[0].summary, b"newer answer");
    }

    #[test]
    fn one_row_per_ref_ordered_by_the_opening_that_survived() {
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:w","ref":"r1","summary":"first"}"#,
            r#"{"ts":"t2","actor":"a:lead","action":"review","target":"a:w","ref":"r2","summary":"second"}"#,
            r#"{"ts":"t3","actor":"a:lead","action":"ask","target":"a:w","ref":"r1","summary":"reopened"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows.len(), 2, "one row per ref");
        assert_eq!(rows[0].kind, b"review", "review opens a request too");
        assert_eq!(rows[1].kind, b"ask");
        // THE ORDER IS BY THE SURVIVING OPENING, NOT BY THE FIRST ONE. r1 was
        // asked at t1 and re-asked at t3; the row keeps the t3 opening, and its
        // POSITION follows t3 too — so r2 (t2) precedes r1 (t3) even though r1's
        assert_eq!(rows[0].id, b"r2");
        assert_eq!(rows[1].id, b"r1");
        assert_eq!(
            rows[1].summary, b"reopened",
            "the newest opening for a ref wins"
        );
    }

    #[test]
    fn a_line_without_a_ref_or_a_brace_is_not_a_request() {
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:w","summary":"no ref"}"#,
            r"not json at all",
            r#" {"ts":"t2","actor":"a:lead","action":"ask","ref":"r9","summary":"leading space"}"#,
            r#"{"ts":"t3","actor":"a:lead","action":"state","ref":"working","summary":"not a request"}"#,
        ]);
        assert!(states(&body).is_empty());
    }

    #[test]
    fn an_unterminated_tail_is_glued_to_the_line_before_it_and_not_dropped() {
        // The measured `tail -r` framing, at this surface: the remainder runs
        // into the previous line, so BOTH are lost as separate records — the
        // request that was on the previous line disappears.
        let mut body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:w","ref":"r1","summary":"survives"}"#,
            r#"{"ts":"t2","actor":"a:lead","action":"ask","target":"a:w","ref":"r2","summary":"eaten"}"#,
        ]);
        body.extend_from_slice(br#"{"ts":"t3","actor":"a:lead","action":"as"#);
        let rows = states(&body);
        assert_eq!(rows.len(), 1, "r2 was consumed by the glue: {rows:?}");
        assert_eq!(rows[0].id, b"r1");
    }

    #[test]
    fn mine_and_inbox_select_opposite_sides_of_the_same_row() {
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"q"}"#,
        ]);
        let lead = Viewer {
            slot: "main".to_owned(),
            session: "s".to_owned(),
            display: "someone-else".to_owned(),
        };
        let worker = Viewer {
            slot: "worker.0".to_owned(),
            session: "s".to_owned(),
            display: "someone-else".to_owned(),
        };
        assert!(text(&table(&body, Mode::Mine, &lead)).contains("r1"));
        assert_eq!(table(&body, Mode::Inbox, &lead), header());
        assert!(text(&table(&body, Mode::Inbox, &worker)).contains("r1"));
        assert_eq!(table(&body, Mode::Mine, &worker), header());
        // The routing keys decide even though the display ref matches neither.
        assert!(text(&table(&body, Mode::All, &Viewer::default())).contains("r1"));
    }

    #[test]
    fn the_filter_uses_the_same_identity_rule_as_closure() {
        // A CHOICE, not a ruling — see `Request::shown_to`. No corpus row
        // constrains the filter (all 24 mine/inbox rows are refusals), and this
        // pins the choice so a seat can overrule it visibly.
        let keyless = container(&[ASK]);
        // A keyless ROW is a Display identity; a ROUTED viewer is not it, even
        // though the display names agree.
        let routed_viewer = Viewer {
            slot: "main".to_owned(),
            session: "s".to_owned(),
            display: "a:lead".to_owned(),
        };
        assert_eq!(
            table(&keyless, Mode::Mine, &routed_viewer),
            header(),
            "mixed identity does not select a row either"
        );
        // A viewer with no routing keys IS a Display identity, and matches.
        let display_viewer = Viewer {
            slot: String::new(),
            session: String::new(),
            display: "a:lead".to_owned(),
        };
        assert!(text(&table(&keyless, Mode::Mine, &display_viewer)).contains("r1"));
        // And the same viewer does not collect somebody else's row.
        let stranger = Viewer {
            slot: String::new(),
            session: String::new(),
            display: "a:third".to_owned(),
        };
        assert_eq!(table(&keyless, Mode::Mine, &stranger), header());
    }

    #[test]
    fn a_pane_s_readings_become_a_viewer_by_the_frozen_rules() {
        use crate::tmux::ObservedViewer;
        let stamped = ObservedViewer {
            slot: Some("worker.2".to_owned()),
            session: Some("s".to_owned()),
            agent: Some("cl:w".to_owned()),
        };
        assert_eq!(
            Viewer::from_pane(&stamped, "s"),
            Viewer {
                slot: "worker.2".to_owned(),
                session: "s".to_owned(),
                display: "cl:w".to_owned()
            }
        );
        // Cross-session: the display ref carries the helper's @session: prefix,
        // and the routing half is the pane's own session.
        assert_eq!(
            Viewer::from_pane(&stamped, "other").display,
            "@s:cl:w".to_owned()
        );
        assert_eq!(Viewer::from_pane(&stamped, "other").session, "s".to_owned());
        // An unstamped pane, and a tampered slot, are display identities.
        for slot in [None, Some("worker.0x".to_owned()), Some("boss".to_owned())] {
            let unstamped = ObservedViewer {
                slot,
                session: Some("s".to_owned()),
                agent: Some("cl:w".to_owned()),
            };
            let viewer = Viewer::from_pane(&unstamped, "s");
            assert!(viewer.is_known());
            assert_eq!((viewer.slot.as_str(), viewer.session.as_str()), ("", ""));
            assert_eq!(viewer.display, "cl:w");
        }
        // No agent is no identity, whatever else was read.
        let anonymous = ObservedViewer {
            slot: Some("main".to_owned()),
            session: Some("s".to_owned()),
            agent: None,
        };
        assert!(!Viewer::from_pane(&anonymous, "s").is_known());
        assert_eq!(Viewer::from_pane(&anonymous, "s"), Viewer::default());
        // No session is no identity: a real pane always has one, and reading
        // its absence as "own session" would drop the cross-session prefix.
        let sessionless = ObservedViewer {
            slot: Some("main".to_owned()),
            session: None,
            agent: Some("cl:lead".to_owned()),
        };
        assert_eq!(Viewer::from_pane(&sessionless, "s"), Viewer::default());
        assert!(!Viewer::from_pane(&sessionless, "s").is_known());
    }

    #[test]
    fn a_slot_is_the_closed_grammar_and_nothing_near_it() {
        for good in ["main", "worker.0", "worker.12", "spawned.3"] {
            assert!(is_slot(good), "{good}");
        }
        for bad in [
            "",
            "Main",
            "worker",
            "worker.",
            "worker.0x",
            "spawned.-1",
            "main2",
            " main",
        ] {
            assert!(!is_slot(bad), "{bad}");
        }
    }

    #[test]
    fn the_summary_never_carries_a_newline_into_the_row() {
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:w","ref":"r1","summary":"one\ntwo"}"#,
        ]);
        let rendered = table(&body, Mode::All, &Viewer::default());
        assert_eq!(
            text(&rendered).lines().count(),
            2,
            "header plus exactly one row: {}",
            text(&rendered)
        );
        assert!(text(&rendered).contains("one two"));
    }

    #[test]
    fn an_over_long_field_overflows_its_column_instead_of_truncating() {
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"review","target":"a:w","ref":"review-20260820T161305Z-dc302d09","summary":"s"}"#,
        ]);
        assert!(
            text(&table(&body, Mode::All, &Viewer::default()))
                .contains("review-20260820T161305Z-dc302d09 a:lead"),
            "one space after an over-wide id, as the A6 capture shows"
        );
    }

    #[test]
    fn mode_tokens_are_exactly_the_three() {
        assert_eq!(Mode::parse(None), Some(Mode::Mine));
        assert_eq!(Mode::parse(Some("mine")), Some(Mode::Mine));
        assert_eq!(Mode::parse(Some("inbox")), Some(Mode::Inbox));
        assert_eq!(Mode::parse(Some("all")), Some(Mode::All));
        for token in ["", "MINE", "al", "alll", "--json", "-a"] {
            assert_eq!(Mode::parse(Some(token)), None, "{token}");
        }
        assert!(Mode::Mine.needs_identity());
        assert!(Mode::Inbox.needs_identity());
        assert!(!Mode::All.needs_identity());
    }

    #[test]
    fn every_status_has_its_own_token() {
        assert_eq!(Status::Pending.token(), "pending");
        assert_eq!(Status::Replied.token(), "replied");
        assert_eq!(Status::Cancelled.token(), "cancelled");
    }

    #[test]
    fn the_opening_fields_the_sensor_carries_are_all_read() {
        // The sensor's row is wider than the table. `ts` and `body_file` are
        // read by the archive digest from the same rows, so a port that dropped
        // them would silently narrow a shared sensor.
        let body = container(&[
            r#"{"ts":"2026-08-20T16:12:55Z","actor":"a:lead","action":"ask","target":"a:w","ref":"r1","body_file":"/m/r1.ask.txt","actor_slot":"main","target_slot":"worker.0","actor_session":"s","target_session":"s","summary":"q"}"#,
        ]);
        let row = &states(&body)[0];
        assert_eq!(row.at, b"2026-08-20T16:12:55Z");
        assert_eq!(row.body_file, b"/m/r1.ask.txt");
        assert_eq!(row.from_slot, Key::Value(b"main".to_vec()));
        assert_eq!(row.to_slot, Key::Value(b"worker.0".to_vec()));
        assert_eq!(row.from_session, Key::Value(b"s".to_vec()));
        assert_eq!(row.to_session, Key::Value(b"s".to_vec()));
        // And the three states are told apart on a published row, which is the
        // whole reason these are Keys and not bytes.
        let mixed = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:w","ref":"r2","actor_slot":"","summary":"q"}"#,
        ]);
        let row = &states(&mixed)[0];
        assert_eq!(row.from_slot, Key::Empty, "present and empty");
        assert_eq!(row.from_session, Key::Absent, "not in the record at all");
        assert_eq!(row.from_slot.value(), Some(b"".as_slice()));
        assert_eq!(row.from_session.value(), None);
    }
}
