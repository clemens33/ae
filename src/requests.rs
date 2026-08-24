//! The `requests` read surface — SC-212c, SC-518, SC-1306d, SC-211d.
//!
//! Two things, kept apart the way the frozen implementation keeps them apart:
//! the request SENSOR ([`states`]) and the TABLE the helper prints
//! ([`render`]). The frozen `requests` helper carries `_ar_request_states`
//! beside it by `declare -f` rather than owning a second copy, and the frozen
//! source says why — the two copies had already drifted once, one checking both
//! ends of a reply and the other only the actor end. So the sensor is a public
//! function here, not a private detail of the table.
//!
//! # The frozen argv, and the successor spelling
//!
//! The corpus rows invoke a per-session script: `<AE_HOME>/sessions/tg1/requests
//! all`. The successor spelling ruled for it (lead ruling on D1, 2026-08-24) is
//!
//! ```text
//! <AE_HOME>/sessions/<name>/requests [mine|inbox|all]
//!   ->  ae _requests <AE_HOME>/sessions/<name> [mine|inbox|all]
//! ```
//!
//! The leading underscore is load-bearing: `_validate_session_name` forbids a
//! leading `_`, so an underscore-prefixed subcommand can never shadow a legal
//! session name and SC-022's "a top-level bare word is a launch candidate"
//! loses nothing. This mapping is a FIXED DECLARED INPUT to a parity run, not
//! something a runner improvises per row — phase-4 criterion 14 requires the
//! effective normalised argv of every invocation.
//!
//! # What this surface deliberately does not do
//!
//! `mine` and `inbox` filter against the CALLER's pane identity, which the
//! frozen helper reads from tmux through `ae_current_agent_ref` and
//! `ae_current_slot`. That sensor is a different ownership record and has not
//! flipped, so this build supplies an empty [`Viewer`] and both modes take the
//! frozen "could not detect current agent identity" refusal. That is not a stub
//! answer dressed as a real one: the message states exactly what is true of this
//! build, it is the same refusal the frozen helper gives outside a pane, and all
//! 24 corpus rows for those two modes pin `rc=1` with those bytes. The typed API
//! takes a `Viewer` so the identity slice can supply one without touching this
//! module.
//!
//! # Pending, replied, cancelled
//!
//! SC-518 is the whole of the pairing rule and the reason the sensor is subtle:
//!
//! - the NEWEST `ask`/`review` per `ref` opens the request (newest-first scan,
//!   first one seen wins);
//! - a `reply` closes it only on a FULL MIRROR — routing identities
//!   (slot + session) when the request's target slot and the reply's actor slot
//!   are both present, display names when they are not. A reply carrying the
//!   right ref from the right agent but addressed elsewhere leaves the request
//!   `pending`, because a loud false-pending beats a silent false-closure;
//! - a `cancel` closes it when its actor is the request's own sender, and a
//!   VALID cancel is terminal even against a later reply;
//! - every candidate of each kind is retained and validated afterwards. Keeping
//!   only the newest raw one lets an INVALID newer event discard a VALID older
//!   one — measured in the frozen tree: ask, valid withdrawal, then a stranger's
//!   cancel rendered `pending`.
//!
//! # WHAT THE PAIRING RULE DOES **NOT** SETTLE
//!
//! SC-518 as ratified says a MIXED pair — one side routed, the other
//! display-only — matches nothing, and [`crate::session`]'s reader implements
//! exactly that for the `list` attention consumer. **The frozen captures for
//! THIS surface disagree with the row**, and the fixtures built to measure it
//! (the `A7` 405j pair matrix, `G5/m6-mixed-routed-display`,
//! `G5/m2-wrong-ref`) pin the disagreement across twelve corpus rows. That is a
//! row-versus-capture conflict escalated to the seats that ratified SC-518,
//! whose own row still reads `Empirical: pending (… + C-cluster)` — the
//! C-cluster being that evidence. It is asserted, shape by shape, in
//! `tests/it/helper_corpus.rs`, not described here, so a ruling either way fails
//! a test rather than needing someone to remember this paragraph.
//!
//! Three shapes are unpinned in BOTH directions and this module's behavior in
//! them is a CHOICE, not a contract: the inverse mixed pair (display-only
//! opening, routed reply), a valid reply to an opening that is later re-asked,
//! and every `cancel` causality question — the corpus contains no inverse-mixed
//! pair, no re-asked ref, and not one `cancel` event. The choice made here is
//! the symmetric one (the display fallback applies in both directions, and
//! terminal candidates are bounded by identity rather than by position),
//! because a rule that is strict one way and loose the other is a third
//! behavior nobody ruled.
//!
//! # SC-1306d
//!
//! The scan is snapshot-semantic. This module reads the container once and
//! answers from those bytes, so a reply appended after the read leaves this
//! invocation's row `pending` and a clean rerun reports `replied`.

use std::collections::HashMap;
use std::path::Path;

use crate::event_text::{
    CONTAINER, event_line, extract, pad_left_aligned, read_container, read_lines, reversed,
};

/// `requests [mine|inbox|all]` — SC-212c's signature, defaulting to `mine`.
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
///
/// Empty [`Viewer::display`] is "could not detect", which is the state this
/// build is always in; see the module docs.
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
    ///
    /// The frozen helper tests `-z "$self"` — the DISPLAY ref — and refuses
    /// before printing the header. A slot without a display ref is not an
    /// identity for this test, which is why this reads one field and not three.
    #[must_use]
    pub fn is_known(&self) -> bool {
        !self.display.is_empty()
    }
}

/// One request, as the sensor emits it.
///
/// The frozen sensor emits a unit-separated row — `\x1f`, deliberately not tabs,
/// because tab is an IFS *whitespace* character and an empty field between two
/// tabs silently shifts every later field left. A struct has no framing to get
/// wrong, so the separator does not survive the port; the FIELD SET and its
/// order do, and the row's own reason for existing (`summary` last, because free
/// text with no separator in it must land intact) is a property of the frozen
/// serialisation rather than of the data.
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
    pub from_slot: Vec<u8>,
    /// Routing key of the target.
    pub to_slot: Vec<u8>,
    /// Session of the sender's routing key.
    pub from_session: Vec<u8>,
    /// Session of the target's routing key.
    pub to_session: Vec<u8>,
    /// The DISPLAY summary: the request's own text while pending, the closing
    /// event's text once closed.
    pub summary: Vec<u8>,
}

/// The three terminal states of a request — SC-518.
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
    ///
    /// Slot when BOTH the row's stored slot and the viewer's slot are nonempty —
    /// which survives a display-name change — and display names otherwise. The
    /// "both" is not a convenience: a row with no routing keys predates them,
    /// and comparing an empty slot to an empty slot would match every such row
    /// to every viewer.
    #[must_use]
    pub fn shown_to(&self, mode: Mode, viewer: &Viewer) -> bool {
        match mode {
            Mode::All => true,
            Mode::Mine => {
                Self::side_matches(&self.from_slot, &self.from_session, &self.from, viewer)
            }
            Mode::Inbox => Self::side_matches(&self.to_slot, &self.to_session, &self.to, viewer),
        }
    }

    fn side_matches(slot: &[u8], session: &[u8], display: &[u8], viewer: &Viewer) -> bool {
        if slot.is_empty() || viewer.slot.is_empty() {
            return display == viewer.display.as_bytes();
        }
        slot == viewer.slot.as_bytes() && session == viewer.session.as_bytes()
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

/// The refusal for `mine`/`inbox` when no identity could be detected.
///
/// Frozen bytes; the 24 A6 corpus rows pin them together with `rc=1`.
pub const NO_IDENTITY: &str =
    "Error: could not detect current agent identity; use 'requests all' outside an ae pane";

/// The exit status of the identity refusal.
///
/// **PINNED, not chosen.** 24 corpus rows record `rc=1` for it. The crate's
/// usage code stays `2` and belongs to the bad-mode token, which no corpus row
/// exercises — that split is the lead's D2 ruling, and it is why this constant
/// lives here rather than being folded into the usage path.
pub const EXIT_NO_IDENTITY: u8 = 1;

/// The header line, byte for byte.
///
/// Not a literal: the same [`write_row`] the rows go through, so a change to the
/// column widths cannot move the rows without moving the header with them.
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
///
/// Infallible on purpose. The frozen sensor tests `[[ -f "$file" ]]` and returns
/// no rows when the container is absent, and its reader is
/// `_ae_tac "$file" 2>/dev/null || true` — so an existing container that cannot
/// be read produces no rows, no diagnostic and `rc=0`, exactly like an absent
/// one. That is SC-519's quiet-empty direction for this surface and NOT
/// SC-509b's degraded direction, which belongs to the typed reader in
/// [`crate::events`]: this table has nowhere to publish a degradation and the
/// frozen bytes show none.
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
///
/// Reads `container` through the reversed framing the frozen helper reads it
/// through (see [`crate::event_text::reversed`]), so the scan is newest-first
/// and the opening `ask` — the oldest record for a ref — is reached last, which
/// is exactly why every terminal candidate has to be retained rather than
/// decided on sight.
#[must_use]
pub fn states(container: &[u8]) -> Vec<Request> {
    let stream = reversed(container);
    let mut opened: HashMap<Vec<u8>, Opening> = HashMap::new();
    let mut replies: HashMap<Vec<u8>, Vec<Closing>> = HashMap::new();
    let mut cancels: HashMap<Vec<u8>, Vec<Closing>> = HashMap::new();
    // Newest first, because that is the order they are met in.
    let mut refs: Vec<Vec<u8>> = Vec::new();

    for line in read_lines(&stream) {
        let Some(line) = event_line(line) else {
            continue;
        };
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
                opened.insert(reference.clone(), Opening::read(line, action));
                refs.push(reference);
            }
            b"reply" => replies
                .entry(reference)
                .or_default()
                .push(Closing::read(line)),
            b"cancel" => cancels
                .entry(reference)
                .or_default()
                .push(Closing::read(line)),
            _ => {}
        }
    }

    // Reversing the newest-first encounter order gives chronological output.
    refs.reverse();
    refs.into_iter()
        .filter_map(|reference| {
            let opening = opened.remove(&reference)?;
            let no_candidates = Vec::new();
            let cancel = cancels
                .get(&reference)
                .unwrap_or(&no_candidates)
                .iter()
                .find(|candidate| opening.withdrawn_by(candidate));
            let reply = replies
                .get(&reference)
                .unwrap_or(&no_candidates)
                .iter()
                .find(|candidate| opening.answered_by(candidate));
            // A valid withdrawal wins over any reply, however late: a straggler
            // answer must not reopen a request nobody is waiting on.
            let (status, summary) = match (cancel, reply) {
                (Some(closing), _) => (Status::Cancelled, closing.summary.clone()),
                (None, Some(closing)) => (Status::Replied, closing.summary.clone()),
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

/// The `ask`/`review` that opened a request.
struct Opening {
    kind: Vec<u8>,
    from: Vec<u8>,
    to: Vec<u8>,
    at: Vec<u8>,
    body_file: Vec<u8>,
    from_slot: Vec<u8>,
    to_slot: Vec<u8>,
    from_session: Vec<u8>,
    to_session: Vec<u8>,
    summary: Vec<u8>,
}

/// A candidate `reply` or `cancel`. One shape for both: a cancel simply has no
/// target side, and reading the fields it does not carry as empty is what the
/// frozen sensor does too.
struct Closing {
    actor: Vec<u8>,
    target: Vec<u8>,
    actor_slot: Vec<u8>,
    target_slot: Vec<u8>,
    actor_session: Vec<u8>,
    target_session: Vec<u8>,
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
            from_slot: extract(line, "actor_slot"),
            to_slot: extract(line, "target_slot"),
            from_session: extract(line, "actor_session"),
            to_session: extract(line, "target_session"),
            summary: fold_newlines(extract(line, "summary")),
        }
    }

    /// SC-518's full mirror: the reply's actor is this request's TARGET and its
    /// target is this request's SENDER.
    fn answered_by(&self, reply: &Closing) -> bool {
        if self.to_slot.is_empty() || reply.actor_slot.is_empty() {
            return reply.actor == self.to && reply.target == self.from;
        }
        reply.actor_slot == self.to_slot
            && reply.actor_session == self.to_session
            && reply.target_slot == self.from_slot
            && reply.target_session == self.from_session
    }

    /// Only the request's own SENDER may withdraw it. A cancel has no target
    /// side to verify, so this is the actor half of the same question.
    fn withdrawn_by(&self, cancel: &Closing) -> bool {
        if self.from_slot.is_empty() || cancel.actor_slot.is_empty() {
            return !cancel.actor.is_empty() && cancel.actor == self.from;
        }
        cancel.actor_slot == self.from_slot && cancel.actor_session == self.from_session
    }
}

impl Closing {
    fn read(line: &[u8]) -> Self {
        Self {
            actor: extract(line, "actor"),
            target: extract(line, "target"),
            actor_slot: extract(line, "actor_slot"),
            target_slot: extract(line, "target_slot"),
            actor_session: extract(line, "actor_session"),
            target_session: extract(line, "target_session"),
            summary: fold_newlines(extract(line, "summary")),
        }
    }
}

/// The frozen sensor's `${_EV//$'\n'/ }` on every summary it stores.
///
/// PROVABLY A NO-OP through the frozen extractor, which already turns a `\n`
/// escape into a space, and a RAW newline inside a value would have ended the
/// line before the extractor ever saw it. Kept because the frozen code keeps
/// it and because this module's summaries are the last field of a line-framed
/// row: the day a value reaches here with a newline in it, one newline must not
/// become two rows.
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
        EXIT_NO_IDENTITY, Mode, NO_IDENTITY, Status, Viewer, header, render, states, table,
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
        // because a header derived from the code under test asserts nothing.
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
        assert!(
            out.stderr.is_empty(),
            "SC-519: absent is quiet, not degraded"
        );
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
    fn sc_518_a_keyless_request_falls_back_to_display_names() {
        // The "both sides" guard, from the other direction: a request that
        // predates routing keys must still close on names, and must not be
        // matched to everything by two empty slots comparing equal.
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"worker.0","actor_session":"s","target_slot":"main","target_session":"s","summary":"routed reply"}"#,
        ]);
        assert_eq!(states(&body)[0].status, Status::Replied);
    }

    #[test]
    fn sc_518_a_valid_cancel_is_terminal_against_a_later_reply() {
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:lead","action":"cancel","ref":"r1","summary":"never mind"}"#,
            r#"{"ts":"t3","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"too late"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows[0].status, Status::Cancelled);
        assert_eq!(rows[0].summary, b"never mind");
    }

    #[test]
    fn sc_518_only_the_sender_may_withdraw() {
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"cancel","ref":"r1","summary":"not yours"}"#,
        ]);
        assert_eq!(states(&body)[0].status, Status::Pending);
        // An anonymous cancel is not a withdrawal either.
        let anonymous = container(&[
            ASK,
            r#"{"ts":"t2","action":"cancel","ref":"r1","summary":"nobody"}"#,
        ]);
        assert_eq!(states(&anonymous)[0].status, Status::Pending);
    }

    #[test]
    fn sc_518_an_invalid_newer_candidate_cannot_bury_a_valid_older_one() {
        // The measured regression the frozen sensor's retain-then-validate
        // shape exists to prevent: keeping only the newest raw cancel made this
        // render `pending`.
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:lead","action":"cancel","ref":"r1","summary":"valid withdrawal"}"#,
            r#"{"ts":"t3","actor":"a:stranger","action":"cancel","ref":"r1","summary":"stranger cancel"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows[0].status, Status::Cancelled);
        assert_eq!(rows[0].summary, b"valid withdrawal");

        // And the same shape with two replies.
        let replies = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"valid answer"}"#,
            r#"{"ts":"t3","actor":"a:stranger","action":"reply","target":"a:lead","ref":"r1","summary":"stranger answer"}"#,
        ]);
        let rows = states(&replies);
        assert_eq!(rows[0].status, Status::Replied);
        assert_eq!(rows[0].summary, b"valid answer");
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
        // earliest ask is the oldest event in the container. Ordinary logs never
        // show this, because a ref is opened once and the ordering collapses to
        // chronological.
        //
        // Not derived from a reading: the frozen `_ar_request_states`, extracted
        // at 72c7293 and run over these exact bytes, emits r2 before r1. The
        // corpus does not cover a re-asked ref, so this expectation is IS from a
        // source-plus-execution read of the frozen sensor and NOT from a capture.
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
    fn a_keyless_row_filters_by_display_name() {
        let body = container(&[ASK]);
        let lead = Viewer {
            slot: "main".to_owned(),
            session: "s".to_owned(),
            display: "a:lead".to_owned(),
        };
        assert!(
            text(&table(&body, Mode::Mine, &lead)).contains("r1"),
            "an empty stored slot falls back to the name, not to nothing"
        );
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
        assert_eq!(row.from_slot, b"main");
        assert_eq!(row.to_slot, b"worker.0");
        assert_eq!(row.from_session, b"s");
        assert_eq!(row.to_session, b"s");
    }
}
