//! `ae compact [name]`: R7 gate, R2 checkpoint, R10 dispatch, 4a audit, P1 report.
//! One nonblocking run lock, no run state (R1); spawned skipped (R3); own
//! checkpoints cancelled, foreign refused (R4). The paste is VERBATIM (no marker:
//! `/` must lead). The ref is pre-computed; the R10 call is MATCHED, never `?`.

use crate::deliver::{self, GuardedRequest};
use crate::error::Result;
use crate::lifecycle;
use crate::meta::{self, Meta};
use crate::seatcompact::{self, GapLeg, Gate, Mode, Outcome, Verdict};
use crate::state;
use crate::store;
use crate::time::Timestamp;
use crate::tool::ToolKind;
use crate::tracked;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

/// The run lock, held whole-run, never unlinked or written into.
pub(crate) const LOCK_NAME: &str = "seatcompact.lock";
/// The bounded settle between a dispatch and the next seat's first step.
pub(crate) const SEAT_SETTLE: Duration = Duration::from_mins(1);
/// The checkpoint-wait poll.
const WAIT_POLL: Duration = Duration::from_secs(2);
/// The R13 actor namespace this verb opens checkpoint requests as.
const ACTOR_PREFIX: &str = "ae:seats:";
/// The skip word for a checkpoint that never closed.
const CHECKPOINT_TIMEOUT: &str = "checkpoint timeout";

/// Run the verb over the session `name` under `root`.
pub(crate) fn run(
    root: &Path,
    name: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let dir = crate::inventory::Roots::under(root).sessions().join(name);
    let lock_path = dir.join(LOCK_NAME);
    let _held = match store::lock(&lock_path, Duration::ZERO) {
        Ok(held) => held,
        Err(why) if why.kind() == std::io::ErrorKind::WouldBlock => {
            writeln!(
                err,
                "another ae compact holds {}",
                seatcompact::cell(&lock_path.to_string_lossy())
            )?;
            return Ok(state::EXIT_FAILED);
        }
        Err(why) => {
            // The lockpath refusal embeds the RAW path; print the projected
            // path plus the leg it names, never the raw bytes.
            let raw = why.to_string();
            let path_text = lock_path.to_string_lossy();
            let leg = raw.strip_prefix(path_text.as_ref()).unwrap_or(raw.as_str());
            writeln!(err, "ae compact: {}{leg}", seatcompact::cell(&path_text))?;
            return Ok(state::EXIT_FAILED);
        }
    };
    let meta_bytes = meta::read_bytes(&dir).unwrap_or_default();
    let uuid = session_uuid_of(&meta_bytes);
    if uuid.is_empty() {
        writeln!(
            err,
            "ae compact: session '{name}' records no session id — refresh or migrate the session, then retry"
        )?;
        return Ok(state::EXIT_FAILED);
    }
    let actor = format!("{ACTOR_PREFIX}{uuid}");
    let storage = store::open(&dir);
    for line in seatcompact::audit_warning(&storage.container(), &actor, Timestamp::now()) {
        writeln!(err, "{line}")?;
    }
    let run = tracked::request_id("seats", Timestamp::now(), entropy());
    append(
        &storage,
        &seatcompact::run_record(Timestamp::now(), &actor, &run, seatcompact::RUN_START),
    )?;
    let text = String::from_utf8_lossy(&meta_bytes).into_owned();
    let parsed = Meta::parse(&text);
    let roster = parsed.roster();
    let mut outcomes: Vec<SeatOutcome> = Vec::new();
    let total = roster.len();
    for (index, entry) in roster.iter().enumerate() {
        let started = Instant::now();
        let mut seat = run_seat(root, name, &dir, &actor, entry, err)?;
        seat.elapsed = i64::try_from(started.elapsed().as_secs()).unwrap_or(i64::MAX);
        append(
            &storage,
            &seatcompact::seat_record(&seatcompact::SeatRecord {
                ts: Timestamp::now(),
                actor: &actor,
                run: &run,
                request: &seat.request,
                slot: &seat.slot,
                session: name,
                outcome: &seat.outcome,
                sanitized: seat.sanitized,
            }),
        )?;
        let dispatched = matches!(seat.outcome, Outcome::Dispatched { .. });
        outcomes.push(seat);
        if dispatched && index + 1 < total {
            std::thread::sleep(SEAT_SETTLE);
        }
    }
    append(
        &storage,
        &seatcompact::run_record(Timestamp::now(), &actor, &run, seatcompact::RUN_END),
    )?;
    let lines: Vec<seatcompact::SeatLine<'_>> = outcomes
        .iter()
        .map(|seat| seatcompact::SeatLine {
            slot: &seat.slot,
            tool: &seat.tool,
            gate: seat.gate,
            outcome: &seat.outcome,
            elapsed: seat.elapsed,
            earlier_checkpoint: None,
        })
        .collect();
    write!(out, "{}", seatcompact::report(&lines))?;
    Ok(0)
}

/// One seat's turn.
struct SeatOutcome {
    slot: String,
    tool: String,
    gate: Gate,
    outcome: Outcome,
    elapsed: i64,
    request: String,
    sanitized: usize,
}

/// Gate, checkpoint, dispatch for one seat: every arm advances or skips.
#[allow(
    clippy::too_many_lines,
    reason = "the seat pipeline, one arm per step; splitting it would scatter the order R1 pins"
)]
fn run_seat(
    root: &Path,
    name: &str,
    dir: &Path,
    actor: &str,
    entry: &meta::RosterEntry,
    err: &mut impl Write,
) -> Result<SeatOutcome> {
    let binary = entry.binary.clone().unwrap_or_default();
    let adapter = ToolKind::from_binary_name(&binary).adapter();
    let spec = adapter.compact;
    let gate = seatcompact::classify(
        spec,
        adapter.input.model,
        entry.slot.starts_with("spawned."),
    );
    let mut seat = SeatOutcome {
        slot: entry.slot.clone(),
        tool: adapter.name.to_owned(),
        gate,
        outcome: Outcome::skipped(""),
        elapsed: 0,
        request: String::new(),
        sanitized: 0,
    };
    let mode = match gate {
        Gate::Admit(mode) => mode,
        Gate::Refuse { reason, .. } => {
            seat.outcome = Outcome::skipped(reason);
            return Ok(seat);
        }
    };
    let Ok((resolved, server)) = tracked::resolve_on(&entry.name, name, dir) else {
        seat.outcome = Outcome::identity_gap(GapLeg::Unreadable);
        return Ok(seat);
    };
    let triple = match tracked::observe_triple(&server, &resolved.pane, dir) {
        Ok(triple) => triple,
        Err(gap) => {
            seat.outcome = Outcome::identity_gap(observe_leg(gap));
            return Ok(seat);
        }
    };
    let storage = store::open(dir);
    cancel_own_pending(&storage, actor, &entry.slot, name)?;
    let goal = storage.goal()?.unwrap_or_default();
    let memo = storage.memo_bytes_or_empty();
    let states = crate::requests::states(&storage.container());
    let refs = open_refs(&states, &entry.slot, name);
    let prepared =
        match crate::sanitize::prepare_body(&goal, latest_decision(&memo), &entry.name, &refs) {
            Ok(prepared) => prepared,
            Err(field) => {
                seat.outcome = Outcome::skipped(&format!("record not utf-8: {}", field.name()));
                return Ok(seat);
            }
        };
    seat.sanitized = prepared.sanitized;
    // The ref the open below WILL mint: `request_id` is deterministic over
    // the `(now, entropy)` this caller supplies.
    let now = Timestamp::now();
    let entropy = entropy();
    let expected = tracked::request_id(tracked::Kind::Ask.id_prefix(), now, entropy);
    let boundary = storage.container().len();
    let baseline = memo.len();
    let body = compose_body(&expected, &prepared, baseline, &triple, dir);
    let sender = tracked::Sender {
        display: actor.to_owned(),
        slot: String::new(),
        session: String::new(),
    };
    let mut sink = Vec::new();
    let code = tracked::run(
        tracked::Kind::Ask,
        dir,
        &[entry.name.clone(), body],
        Some(&sender),
        name,
        now,
        entropy,
        Duration::ZERO,
        &mut sink,
        err,
    )?;
    if code != 0 {
        // The pane was resolvable moments ago; a refusal past that is
        // occupancy unless liveness now proves death.
        seat.outcome =
            match deliver::observe_pane_liveness(&server, dir, &resolved.pane, &resolved.slot) {
                deliver::PaneLiveness::Dead => Outcome::skipped(seatcompact::DEAD),
                deliver::PaneLiveness::Alive | deliver::PaneLiveness::Unproven => {
                    Outcome::skipped(seatcompact::BUSY)
                }
            };
        return Ok(seat);
    }
    let found = crate::requests::states(&storage.container())
        .into_iter()
        .find(|req| {
            req.id == expected.as_bytes() && req.status == crate::requests::Status::Pending
        });
    let Some(opened) = found else {
        writeln!(
            err,
            "ae compact: checkpoint {expected} is unrecorded — seat left alone"
        )?;
        seat.outcome = Outcome::skipped(CHECKPOINT_TIMEOUT);
        return Ok(seat);
    };
    seat.request.clone_from(&expected);
    let to_slot = opened.to_slot.value().unwrap_or_default();
    let to_session = opened.to_session.value().unwrap_or_default();
    let deadline = Instant::now() + Duration::from_secs(crate::compact::DEFAULT_HANDOVER_SECS);
    let facts = wait_for_facts(
        dir, &expected, to_slot, to_session, &opened.to, boundary, baseline, deadline,
    );
    let Some((reply, memo_caller)) = facts else {
        seat.outcome = Outcome::skipped(CHECKPOINT_TIMEOUT);
        return Ok(seat);
    };
    let paste = match (mode, spec) {
        (Mode::Guided, crate::tool::CompactSpec::Guided { command }) => {
            format!("{command} checkpoint {expected} saved; compact now, then re-read `ae brief`")
        }
        (Mode::Bare, crate::tool::CompactSpec::Bare { command }) => command.to_owned(),
        _ => String::new(), // The gate refused every `Unsupported` seat.
    };
    let request = GuardedRequest {
        dir,
        server: &server,
        pane: &resolved.pane,
        pane_slot: &resolved.slot,
        target_session: name,
        own_session: name,
        model: adapter.input.model,
        text: &paste,
        defer: deliver::DEFAULT_DEFER,
    };
    let carried = Carried {
        slot: entry.slot.clone(),
        session: name.to_owned(),
        uuid: session_uuid_of(&meta::read_bytes(dir).unwrap_or_default()),
        reply,
        memo: memo_caller,
    };
    // MATCHED into the vocabulary, never `?`: `EnterFailed` is an outcome.
    seat.outcome = match deliver::deliver_guarded(&request, || {
        let guard = lifecycle::lock(root, name).map_err(|_| deliver::Leg::LifecycleLocked)?;
        let Some(seen) = crate::transport::observe_viewer(&server, &resolved.pane) else {
            return Err(deliver::Leg::Unreadable);
        };
        judge_viewer(&seen, &resolved.pane, &carried)?;
        Ok(guard)
    }) {
        Ok(deliver::Outcome::Sent(state)) => {
            Outcome::from_verdict(Verdict::State(state), &resolved.pane)
        }
        Ok(deliver::Outcome::Skipped(leg)) => skipped_leg(leg),
        Err(deliver::EnterFailed) => Outcome::from_verdict(Verdict::EnterFailed, &resolved.pane),
    };
    Ok(seat)
}

/// Pre-lock facts for the under-lock proof (meta id carried; liveness re-taken).
struct Carried {
    slot: String,
    session: String,
    uuid: String,
    reply: tracked::IdentityTriple,
    memo: tracked::IdentityTriple,
}

/// Under-lock proof over one live viewer observation; each leg separately.
fn judge_viewer(
    seen: &crate::tmux::ObservedViewer,
    pane: &str,
    carried: &Carried,
) -> std::result::Result<(), deliver::Leg> {
    let live_uuid = match &seen.session_uuid {
        crate::tmux::OptionReading::Set(value) => {
            let canonical = crate::archive::canonical_uuid(value);
            if canonical.is_empty() {
                return Err(deliver::Leg::Mismatch);
            }
            canonical
        }
        crate::tmux::OptionReading::Vacant => return Err(deliver::Leg::Vacant),
        crate::tmux::OptionReading::Unknown => return Err(deliver::Leg::Unreadable),
    };
    if live_uuid != carried.uuid {
        return Err(deliver::Leg::Mismatch);
    }
    if seen.session.as_deref() != Some(carried.session.as_str())
        || seen.slot.as_deref() != Some(carried.slot.as_str())
    {
        return Err(deliver::Leg::Live);
    }
    let Some(server) = seen.socket_path.as_deref().filter(|path| !path.is_empty()) else {
        return Err(deliver::Leg::Unreadable);
    };
    let live = tracked::IdentityTriple {
        server: server.to_owned(),
        pane: pane.to_owned(),
        session_uuid: live_uuid,
    };
    if !tracked::caller_matches_live(&carried.reply, &live)
        || !tracked::caller_matches_live(&carried.memo, &live)
    {
        return Err(deliver::Leg::Live);
    }
    Ok(())
}

/// Pre-lock observation gap onto the four R1 legs (`Invalid` is `Mismatch`).
fn observe_leg(gap: tracked::CorrelationGap) -> GapLeg {
    match gap {
        tracked::CorrelationGap::Vacant => GapLeg::Vacant,
        tracked::CorrelationGap::Mismatch | tracked::CorrelationGap::Invalid => GapLeg::Mismatch,
        tracked::CorrelationGap::Unreadable
        | tracked::CorrelationGap::NoSession
        | tracked::CorrelationGap::MetaMissing
        | tracked::CorrelationGap::MetaNonregular
        | tracked::CorrelationGap::MetaUnreadable
        | tracked::CorrelationGap::MetaEmpty
        | tracked::CorrelationGap::MetaMalformed
        | tracked::CorrelationGap::MetaDuplicate => GapLeg::Unreadable,
    }
}

/// Guarded-operation skip onto the vocabulary (constants, never values).
fn skipped_leg(leg: deliver::Leg) -> Outcome {
    match leg {
        deliver::Leg::Dead => Outcome::skipped(seatcompact::DEAD),
        deliver::Leg::Busy => Outcome::skipped(seatcompact::BUSY),
        deliver::Leg::TargetLocked => Outcome::skipped(seatcompact::TARGET_LOCKED),
        deliver::Leg::LifecycleLocked => Outcome::skipped(seatcompact::LIFECYCLE_LOCKED),
        deliver::Leg::PasteFailed => Outcome::skipped(seatcompact::PASTE_FAILED),
        deliver::Leg::Unreadable => Outcome::identity_gap(GapLeg::Unreadable),
        deliver::Leg::Vacant => Outcome::identity_gap(GapLeg::Vacant),
        deliver::Leg::Mismatch => Outcome::identity_gap(GapLeg::Mismatch),
        deliver::Leg::Live => Outcome::identity_gap(GapLeg::Live),
    }
}

/// Cancel this verb's OWN earlier pending checkpoints for the seat (opener
/// byte-equal to the R13 actor); any other opener is left alone.
fn cancel_own_pending(
    storage: &store::SessionStore,
    actor: &str,
    slot: &str,
    session: &str,
) -> Result<()> {
    for req in crate::requests::states(&storage.container()) {
        if req.status != crate::requests::Status::Pending
            || req.from.as_slice() != actor.as_bytes()
            || !key_is(&req.to_slot, slot)
            || !key_is(&req.to_session, session)
        {
            continue;
        }
        let opener = String::from_utf8_lossy(&req.from).into_owned();
        let reference = String::from_utf8_lossy(&req.id).into_owned();
        append(
            storage,
            &state::event_line(
                Timestamp::now(),
                &opener,
                "cancel",
                &reference,
                "withdrawn: superseded by a fresh seat-compact checkpoint",
            ),
        )?;
    }
    Ok(())
}

/// Whether a routing key carries exactly `want`.
fn key_is(key: &crate::requests::Key, want: &str) -> bool {
    key.value().is_some_and(|value| value == want.as_bytes())
}

/// The seat's open request ids (pending, targets-or-sent by it), ledger-ordered.
fn open_refs<'a>(
    states: &'a [crate::requests::Request],
    slot: &str,
    session: &str,
) -> Vec<crate::sanitize::LedgerRef<'a>> {
    states
        .iter()
        .enumerate()
        .filter(|(_, req)| {
            req.status == crate::requests::Status::Pending
                && ((key_is(&req.to_slot, slot) && key_is(&req.to_session, session))
                    || (key_is(&req.from_slot, slot) && key_is(&req.from_session, session)))
        })
        .map(|(position, req)| crate::sanitize::LedgerRef {
            position,
            bytes: &req.id,
        })
        .collect()
}

/// Latest `decision` record bytes (last wins, as `brief` selects; byte-kept).
fn latest_decision(memo: &[u8]) -> &[u8] {
    let mut found: &[u8] = b"";
    for record in crate::memo::records(memo) {
        if record.topic == b"decision" {
            found = record.text;
        }
    }
    found
}

/// The checkpoint body: ref, baseline, target, both facts, `ae brief`
/// instruction (never `memo read`), budgeted goal/decision/ids. INPUT only.
fn compose_body(
    reference: &str,
    prepared: &crate::sanitize::PreparedBody,
    baseline: usize,
    triple: &tracked::IdentityTriple,
    dir: &Path,
) -> String {
    use crate::sanitize::{Budgeted, Field, NameStatus};
    use std::fmt::Write as _;
    let mut body = format!(
        "SEATS CHECKPOINT {reference} — prove durable state before ae compacts this seat.\n\
         1. {}/memo add --topic <yours> \"{reference} <must-re-read>\" (id FIRST, any topic).\n\
         2. Reply with the exact command at the end of this message.\n\
         3. After compaction, re-read `ae brief` (never `memo read`).\n\
         AE-SEATS-MEMO-BASELINE={baseline}\n\
         AE-SEATS-TARGET={}|{}|{}\n",
        dir.display(),
        triple.server,
        triple.pane,
        triple.session_uuid,
    );
    let seat = match &prepared.name {
        NameStatus::Rides(name) => name.clone(),
        NameStatus::Omitted => crate::sanitize::name_marker(),
    };
    let _ = writeln!(body, "\nSeat: {seat}");
    for (field, budgeted) in [
        (Field::Goal, &prepared.goal),
        (Field::Decision, &prepared.decision),
    ] {
        let title = match field {
            Field::Goal => "Goal",
            Field::Decision => "Latest decision checkpoint",
        };
        let text = match budgeted {
            Budgeted::Rides(text) if text.is_empty() => "(none)".to_owned(),
            Budgeted::Rides(text) => text.clone(),
            Budgeted::Omitted { over } => crate::sanitize::omit_marker(field, *over),
        };
        let _ = writeln!(body, "\n{title}:\n{text}");
    }
    let _ = writeln!(body, "\nOpen request ids involving this seat:");
    if prepared.ids.riding.is_empty() {
        let _ = writeln!(body, "(none)");
    }
    for id in &prepared.ids.riding {
        let _ = writeln!(body, "{id}");
    }
    if prepared.ids.invalid > 0 {
        let _ = writeln!(
            body,
            "{}",
            crate::sanitize::invalid_marker(prepared.ids.invalid)
        );
    }
    if prepared.ids.over_cap > 0 {
        let _ = writeln!(
            body,
            "{}",
            crate::sanitize::over_cap_marker(prepared.ids.over_cap)
        );
    }
    body
}

/// Wait for BOTH R2 facts: this ref's reply from the request's own target
/// slot, and the `to` author's memo event carrying the ref, backed by a
/// durable row after the baseline. `None` past the bound; the request stays
/// open. A caller field missing satisfies nothing.
#[allow(
    clippy::too_many_arguments,
    reason = "the waiter's question, spelled out rather than bundled"
)]
fn wait_for_facts(
    dir: &Path,
    id: &str,
    to_slot: &[u8],
    to_session: &[u8],
    to_display: &[u8],
    events_boundary: usize,
    memo_baseline: usize,
    deadline: Instant,
) -> Option<(tracked::IdentityTriple, tracked::IdentityTriple)> {
    let storage = store::open(dir);
    loop {
        let events = storage.container();
        if let Some(reply) = reply_fact(&events, id, to_slot, to_session)
            && let Some(memo) = memo_fact(
                &events,
                &storage.memo_bytes_or_empty(),
                events_boundary,
                memo_baseline,
                id.as_bytes(),
                to_display,
            )
        {
            return Some((reply, memo));
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(WAIT_POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

/// One flat member present AND equal.
fn member_eq(line: &[u8], key: &str, want: &[u8]) -> bool {
    crate::event_text::member(line, key).value() == Some(want)
}

/// Whether `id` appears as a whole whitespace-delimited token.
fn has_token(bytes: &[u8], id: &[u8]) -> bool {
    !id.is_empty() && bytes.split(u8::is_ascii_whitespace).any(|word| word == id)
}

/// One caller field off an event, or `None` when it is missing or empty.
fn caller_field(line: &[u8], key: &str) -> Option<String> {
    let member = crate::event_text::member(line, key);
    let value = member.value().filter(|value| !value.is_empty())?;
    Some(String::from_utf8_lossy(value).into_owned())
}

/// The caller's incarnation off an event, or `None` when any caller field is
/// missing or empty.
fn caller_of(line: &[u8]) -> Option<tracked::IdentityTriple> {
    Some(tracked::IdentityTriple {
        server: caller_field(line, "caller_server")?,
        pane: caller_field(line, "caller_pane")?,
        session_uuid: caller_field(line, "caller_session_uuid")?,
    })
}

/// Fact A: this ref's `reply` from the request's own target slot+session.
fn reply_fact(
    events: &[u8],
    id: &str,
    to_slot: &[u8],
    to_session: &[u8],
) -> Option<tracked::IdentityTriple> {
    for raw in crate::event_text::read_lines(events) {
        let Some(line) = crate::event_text::event_line(raw) else {
            continue;
        };
        if member_eq(line, "action", b"reply")
            && member_eq(line, "ref", id.as_bytes())
            && member_eq(line, "actor_slot", to_slot)
            && member_eq(line, "actor_session", to_session)
            && let Some(caller) = caller_of(line)
        {
            return Some(caller);
        }
    }
    None
}

/// Fact B: the `to` author's memo EVENT (token in summary, past the open
/// boundary) AND its durable row past the memo baseline.
fn memo_fact(
    events: &[u8],
    memo: &[u8],
    events_boundary: usize,
    memo_baseline: usize,
    id: &[u8],
    to_display: &[u8],
) -> Option<tracked::IdentityTriple> {
    let tail = memo.get(memo_baseline..).unwrap_or_default();
    let row = crate::memo::records(tail)
        .any(|record| record.author == to_display && has_token(record.text, id));
    if !row {
        return None;
    }
    let mut offset = 0;
    for raw in crate::event_text::read_lines(events) {
        let start = offset;
        offset += raw.len() + 1;
        if start < events_boundary {
            continue;
        }
        let Some(line) = crate::event_text::event_line(raw) else {
            continue;
        };
        if member_eq(line, "action", b"memo")
            && member_eq(line, "actor", to_display)
            && crate::event_text::member(line, "summary")
                .value()
                .is_some_and(|summary| has_token(summary, id))
            && let Some(caller) = caller_of(line)
        {
            return Some(caller);
        }
    }
    None
}

/// Canonical meta `session_id`, or empty (as `reboot_actor` derives it).
fn session_uuid_of(meta: &[u8]) -> String {
    crate::archive::canonical_uuid(
        &meta::first_value(meta, "session_id")
            .map(|raw| String::from_utf8_lossy(raw).into_owned())
            .unwrap_or_default(),
    )
}

/// Append one audit line; a failure aborts the run (crash semantics).
fn append(storage: &store::SessionStore, line: &str) -> Result<()> {
    storage
        .append_event(line)
        .map_err(|why| std::io::Error::other(why.to_string()))?;
    Ok(())
}

/// Randomness for the checkpoint ref and the run id.
fn entropy() -> u64 {
    use std::hash::{BuildHasher as _, RandomState};
    RandomState::new().hash_one((std::process::id(), Timestamp::now().epoch()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux::{ObservedViewer, OptionReading};

    const ID: &str = "ae-20260917T120000Z-abcdef01";
    const TO: &str = "cl:lead";
    const UUID: &str = "11111111-2222-3333-4444-555555555555";

    fn ev(base: &[(&str, &str)], caller: bool) -> Vec<u8> {
        let mut fields = base.to_vec();
        if caller {
            fields.extend([
                ("caller_server", "/sock"),
                ("caller_pane", "%1"),
                ("caller_session_uuid", UUID),
            ]);
        }
        let body: Vec<String> = fields
            .iter()
            .map(|(k, v)| format!("\"{k}\":\"{v}\""))
            .collect();
        format!("{{{}}}\n", body.join(",")).into_bytes()
    }

    fn reply(reference: &str, slot: &str, caller: bool) -> Vec<u8> {
        ev(
            &[
                ("action", "reply"),
                ("ref", reference),
                ("actor_slot", slot),
                ("actor_session", "s"),
            ],
            caller,
        )
    }

    #[test]
    fn only_a_complete_bound_reply_is_fact_a() {
        let hit = reply(ID, "main", true);
        let caller = reply_fact(&hit, ID, b"main", b"s").expect("the bound reply");
        assert_eq!(caller.server, "/sock");
        assert_eq!(caller.pane, "%1");
        assert_eq!(caller.session_uuid, UUID);
        assert!(reply_fact(&reply(ID, "main", false), ID, b"main", b"s").is_none());
        assert!(reply_fact(&hit, "ae-20260917T120000Z-00000000", b"main", b"s").is_none());
        assert!(reply_fact(&hit, ID, b"worker.0", b"s").is_none());
    }

    #[test]
    fn only_a_bound_row_plus_event_is_fact_b() {
        let row = format!("ts\t{TO}\tgoal\t{ID} keep\n").into_bytes();
        let summary = format!("{ID} keep");
        let event = ev(
            &[("action", "memo"), ("actor", TO), ("summary", &summary)],
            true,
        );
        let events = [event.clone(), event].concat();
        assert!(memo_fact(&events, &row, 0, 0, ID.as_bytes(), TO.as_bytes()).is_some());
        assert!(memo_fact(&events, &row, events.len(), 0, ID.as_bytes(), TO.as_bytes()).is_none());
        assert!(memo_fact(&events, &row, 0, row.len(), ID.as_bytes(), TO.as_bytes()).is_none());
        let untokened = format!("ts\t{TO}\tgoal\tno token here\n").into_bytes();
        assert!(memo_fact(&events, &untokened, 0, 0, ID.as_bytes(), TO.as_bytes()).is_none());
        let silent = ev(
            &[("action", "memo"), ("actor", TO), ("summary", &summary)],
            false,
        );
        assert!(memo_fact(&silent, &row, 0, 0, ID.as_bytes(), TO.as_bytes()).is_none());
        assert!(memo_fact(&events, b"", 0, 0, ID.as_bytes(), TO.as_bytes()).is_none());
    }

    fn carried() -> Carried {
        let triple = tracked::IdentityTriple {
            server: "/sock".to_owned(),
            pane: "%1".to_owned(),
            session_uuid: UUID.to_owned(),
        };
        Carried {
            slot: "main".to_owned(),
            session: "s".to_owned(),
            uuid: UUID.to_owned(),
            reply: triple.clone(),
            memo: triple,
        }
    }

    fn viewer(slot: &str, uuid: OptionReading) -> ObservedViewer {
        ObservedViewer {
            slot: Some(slot.to_owned()),
            session: Some("s".to_owned()),
            agent: Some(TO.to_owned()),
            session_uuid: uuid,
            socket_path: Some("/sock".to_owned()),
        }
    }

    #[test]
    fn the_reproof_names_its_leg() {
        let held = carried();
        let live = viewer("main", OptionReading::Set(UUID.to_owned()));
        assert!(judge_viewer(&live, "%1", &held).is_ok());
        let restamped = viewer("worker.0", OptionReading::Set(UUID.to_owned()));
        assert_eq!(
            judge_viewer(&restamped, "%1", &held),
            Err(deliver::Leg::Live)
        );
        let other = viewer(
            "main",
            OptionReading::Set("22222222-2222-2222-2222-222222222222".to_owned()),
        );
        assert_eq!(
            judge_viewer(&other, "%1", &held),
            Err(deliver::Leg::Mismatch)
        );
    }
}
