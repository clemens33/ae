//! The delivery leg: what actually carries a recorded brief into a pane.
//!
//! Split from the record itself because the two are different domains. The
//! parent is a PURE, fuzzed codec plus the one gate [`super::decide`]; this is
//! the tmux-I/O half — it resolves a target, takes the record lock, proves the
//! seat, and spends an attempt. No grammar and no gate arm lives here, so the
//! half that reads hostile bytes stays testable without a pane.
//!
//! Everything that reaches the pane — the text, and the actor its provenance
//! line names — is read from the record, never from argv, the environment or
//! the caller.

use std::io::Write;
use std::path::Path;

use super::{
    Decision, Facts, MAX_ATTEMPTS, Phase, Record, decide, path, read, render, swap_if_unchanged,
};

/// The action a caller names to reach this leg. It selects the leg and NOTHING
/// else — the text and the actor come from the record — so the worst a forged
/// trigger can do is re-fire a brief the spawner already authorized.
pub const RETRY_ACTION: &str = "brief-retry";

/// The event a landed retry writes.
pub const DELIVERED_ACTION: &str = "brief-delivered";

/// The event a record's end writes, whatever ended it.
pub const GAVE_UP_ACTION: &str = "brief-gave-up";

/// The action the body store names the recovery file after — the SAME one the
/// original spawn used, because this is that spawn's brief, not a new message.
const SPAWN_ACTION: &str = "spawn";

/// How many readiness polls a retry spends. Short on purpose: a cycle that
/// finds the box busy simply looks again next cycle, and spends no attempt.
const RETRY_READY_POLLS: u32 = 4;

/// Deliver the brief `slot`'s record holds, or say why it did not. The argv
/// named only WHICH seat: everything that reaches the pane — the text, and the
/// actor its provenance line names — is read from the record, so no caller can
/// put words in a brief's mouth.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
pub fn run(
    dir: &Path,
    target: &str,
    own_session: &str,
    now: crate::time::Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> std::io::Result<u8> {
    use crate::state::EXIT_FAILED;

    // Every refusal below is the same sentence with a different clause.
    let refuse = |err: &mut dyn Write, clause: &str| -> std::io::Result<u8> {
        writeln!(err, "ae: {RETRY_ACTION} {clause}")?;
        Ok(EXIT_FAILED)
    };
    let (resolved, server) = match crate::tracked::resolve_on(target, own_session, dir) {
        Ok(resolved) => resolved,
        Err(why) => {
            writeln!(err, "{}", why.message())?;
            return Ok(EXIT_FAILED);
        }
    };
    // OWN SESSION ONLY. The record is named from this session's own directory,
    // so another session's target is a mistake or an attempt to aim someone
    // else's brief.
    if !resolved.session.is_empty() && resolved.session != own_session {
        return refuse(
            err,
            &format!(
                "refused — {target} is in session '{}', and a brief is retried only into its own",
                resolved.session
            ),
        );
    }
    if resolved.slot.is_empty() {
        return refuse(err, &format!("refused — {target} carries no slot"));
    }
    // THE RECORD LOCK, held across the whole read-decide-write sequence. A
    // second helper — a restarted daemon's orphan, or a forged trigger — waits,
    // fails and skips, so two flights can never both publish a mark and paste.
    let Ok(_held) = crate::store::lock(&path(dir, &resolved.slot), crate::store::LOCK_WAIT) else {
        let slot = &resolved.slot;
        return refuse(
            err,
            &format!("skipped — another flight holds {slot}'s record"),
        );
    };
    let Some(reading) = read(dir, &resolved.slot) else {
        return refuse(
            err,
            &format!("refused — {target} has no undelivered brief on record"),
        );
    };
    let record = match reading {
        Ok(record) => record,
        // Damage is the sweep's to classify and set aside; a delivery leg that
        // acted on it would be deciding with bytes it could not read.
        Err(damaged) => {
            let (slot, kind) = (&resolved.slot, damaged.kind);
            return refuse(
                err,
                &format!("refused — {slot}'s record is damaged: {kind}"),
            );
        }
    };
    let seat = seat_facts(dir, &resolved.slot);
    let input = seat.tool.adapter().input;
    // The pane the NAME resolves to now. If the slot moved, this is a different
    // pane than the record names, and the gate refuses on that.
    let live_pane = (resolved.slot == record.slot).then(|| resolved.pane.clone());
    let liveness =
        crate::deliver::observe_pane_liveness(&server, dir, &resolved.pane, &resolved.slot);
    let ready = liveness == crate::deliver::PaneLiveness::Alive
        && crate::deliver::wait_input_ready(
            &server,
            &resolved.pane,
            input.model,
            input.composed,
            RETRY_READY_POLLS,
        );
    let facts = Facts {
        meta_name: seat.name.as_deref(),
        meta_launch_id: seat.launch_id.as_deref(),
        live_pane: live_pane.as_deref(),
        liveness,
        ready,
        now: now.epoch(),
    };
    let name = seat.name.as_deref().unwrap_or(target);
    match decide(&record, &facts) {
        Decision::Skip(why) => refuse(err, &format!("skipped for {name} — {why}")),
        Decision::GiveUp(why) => {
            let witness = render(&record);
            give_up(dir, &record, name, why, witness.as_bytes(), now, err)?;
            writeln!(err, "ae: brief for {name} given up — {why}")?;
            Ok(EXIT_FAILED)
        }
        Decision::Deliver => fly(
            &Flight {
                dir,
                server: &server,
                pane: &resolved.pane,
                own_session,
                name,
                record: &record,
                composed: input.composed,
                now,
            },
            out,
            err,
        ),
    }
}

/// What meta says about the seat a slot holds right now. `None` on either
/// field means meta did not answer, which the gate skips on rather than acts.
struct Seat {
    name: Option<String>,
    launch_id: Option<String>,
    /// The seat's tool, for the input grammar readiness is proved against.
    tool: crate::tool::ToolKind,
}

/// Read the seat's facts from ONE read of the meta document. The roster and
/// `launch_id.<slot>` are two lookups over those same bytes, because the
/// roster entry does not carry the launch token.
fn seat_facts(dir: &Path, slot: &str) -> Seat {
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let meta = crate::meta::Meta::parse(&text);
    let entry = meta.roster().iter().find(|entry| entry.slot == slot);
    Seat {
        name: entry.map(|entry| entry.name.clone()),
        launch_id: crate::meta::sole_value(&bytes, &format!("launch_id.{slot}"))
            .map(|value| String::from_utf8_lossy(value).into_owned())
            .filter(|value| !value.is_empty()),
        tool: crate::tool::ToolKind::from_binary_name(
            entry
                .and_then(|entry| entry.binary.as_deref())
                .unwrap_or(""),
        ),
    }
}

/// Everything one flight needs, kept as a struct because inlining it buys a
/// `too_many_arguments` allow and nothing else.
struct Flight<'a> {
    dir: &'a Path,
    server: &'a crate::inventory::ServerId,
    pane: &'a str,
    own_session: &'a str,
    name: &'a str,
    record: &'a Record,
    composed: crate::tool::Composed,
    now: crate::time::Timestamp,
}

/// Publish the flight mark, deliver, and record what happened.
///
/// THE ORDER IS THE PROOF: the bumped attempt and the `pasting` mark are made
/// durable BEFORE the paste, so a crash anywhere past this point leaves a
/// record the next cycle refuses to paste again. Only a failure that proves
/// NOTHING was staged re-arms it.
fn fly(flight: &Flight<'_>, out: &mut impl Write, err: &mut impl Write) -> std::io::Result<u8> {
    use crate::state::EXIT_FAILED;

    let witness = render(flight.record);
    let mut taking_off = flight.record.clone();
    taking_off.attempts = taking_off.attempts.saturating_add(1);
    taking_off.phase = Phase::Pasting;
    let mark = render(&taking_off);
    if let Err(why) = swap_if_unchanged(
        flight.dir,
        &taking_off.slot,
        witness.as_bytes(),
        Some(&taking_off),
    ) {
        writeln!(
            err,
            "ae: brief for {} not attempted — its flight mark could not be published: {why}",
            flight.name
        )?;
        return Ok(EXIT_FAILED);
    }
    let request = crate::deliver::Request {
        dir: flight.dir,
        server: flight.server,
        pane: flight.pane,
        logged_target: flight.name,
        target_session: flight.own_session,
        pane_slot: &flight.record.slot,
        own_session: flight.own_session,
        action: SPAWN_ACTION,
        reference: &flight.record.reference,
        actor: &flight.record.actor,
        body: &flight.record.body,
        shape: crate::deliver::Shape::Launch,
        defer: crate::deliver::DEFAULT_DEFER,
        composed: flight.composed,
    };
    let outcome = crate::deliver::deliver(&request, err)?;
    let age = flight.now.epoch().saturating_sub(flight.record.created);
    match outcome {
        Ok(delivered) => {
            if swap_if_unchanged(flight.dir, &flight.record.slot, mark.as_bytes(), None) == Ok(true)
            {
                let _ = std::fs::remove_file(
                    flight.dir.join(format!("undelivered.{}.txt", flight.name)),
                );
            }
            record_event(
                flight.dir,
                DELIVERED_ACTION,
                flight.record,
                flight.name,
                &format!(
                    "brief delivered on attempt {} after {age}s",
                    taking_off.attempts
                ),
                &delivered.body_file,
                flight.now,
            );
            writeln!(out, "Delivered the undelivered brief to {}", flight.name)?;
            Ok(0)
        }
        // PROVEN pre-stage: deliver refuses these before the first key reaches
        // the pane, so nothing was staged and the record may be armed again.
        Err(
            failure @ (crate::deliver::Failure::DeadPane
            | crate::deliver::Failure::Lock
            | crate::deliver::Failure::Abandoned
            | crate::deliver::Failure::NotComposed { .. }),
        ) => {
            rearm_after_prestage(flight, &taking_off, &mark, &failure, err)?;
            Ok(EXIT_FAILED)
        }
        // Everything else may have staged something. The brief is given up
        // rather than risked twice — the file is kept, so nothing is lost.
        Err(failure) => {
            let why = if matches!(failure, crate::deliver::Failure::Unconfirmed { .. }) {
                "submit unconfirmed; the brief may be staged unsent"
            } else {
                "delivery failed in a way that proves nothing about what was pasted"
            };
            give_up(
                flight.dir,
                &taking_off,
                flight.name,
                why,
                mark.as_bytes(),
                flight.now,
                err,
            )?;
            Ok(EXIT_FAILED)
        }
    }
}

/// Put a record back after a refusal that PROVED nothing was staged. The
/// attempt is already spent — ae entered delivery, which is what the count
/// means — so the record goes back armed with the higher count, and the seat
/// keeps whatever attempts remain. Compare-and-swapped for the successor case
/// [`swap_if_unchanged`] names.
fn rearm_after_prestage(
    flight: &Flight<'_>,
    taking_off: &Record,
    mark: &str,
    failure: &crate::deliver::Failure,
    err: &mut impl Write,
) -> std::io::Result<()> {
    if taking_off.attempts >= MAX_ATTEMPTS {
        return give_up(
            flight.dir,
            taking_off,
            flight.name,
            "delivery was attempted twice",
            mark.as_bytes(),
            flight.now,
            err,
        );
    }
    let mut armed = taking_off.clone();
    armed.phase = Phase::Armed;
    let clause = match swap_if_unchanged(flight.dir, &armed.slot, mark.as_bytes(), Some(&armed)) {
        Ok(true) => {
            format!("refused before anything was pasted ({failure:?}) — it stays on record")
        }
        Ok(false) => {
            "was replaced while its delivery was in the air — nothing was written back".to_owned()
        }
        Err(why) => format!(
            "could not be re-armed ({why}) — its flight mark stands, so it will be given up rather than pasted twice"
        ),
    };
    writeln!(err, "ae: brief for {} {clause}", flight.name)
}

/// End a record: drop it if this flight still owns it, KEEP the preserved
/// `.txt`, and say so in the ledger. The file stays on purpose — a given-up
/// brief is one a human now has to hand over, and the event says so.
fn give_up(
    dir: &Path,
    record: &Record,
    name: &str,
    why: &str,
    witness: &[u8],
    now: crate::time::Timestamp,
    err: &mut impl Write,
) -> std::io::Result<()> {
    if swap_if_unchanged(dir, &record.slot, witness, None) != Ok(true) {
        writeln!(
            err,
            "ae: brief for {name} was replaced before it could be given up — nothing was removed"
        )?;
        return Ok(());
    }
    let age = now.epoch().saturating_sub(record.created);
    record_event(
        dir,
        GAVE_UP_ACTION,
        record,
        name,
        &format!(
            "{why} (attempts {}, age {age}s); the brief is preserved at undelivered.{name}.txt",
            record.attempts
        ),
        "",
        now,
    );
    Ok(())
}

/// Append one brief event, named by the ORIGINAL spawner — never the watchdog
/// and never ae. The authority this brief carries is the one that spawned the
/// seat, and the ledger says what the pane's provenance line does.
fn record_event(
    dir: &Path,
    action: &str,
    record: &Record,
    name: &str,
    summary: &str,
    body_file: &str,
    now: crate::time::Timestamp,
) {
    let _ = crate::store::open(dir).append_event(&crate::tracked::event_line(
        &crate::tracked::EventFields::new(
            now,
            &record.actor,
            action,
            name,
            &record.reference,
            "",
            "",
            &record.slot,
            "",
            summary,
            body_file,
        ),
    ));
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "a fixture inspects the real directory the leg wrote; the capability                   boundary is about what PRODUCT code may reach, which is why the                   inventory in tests/it/phase3.rs counts product lines only"
    )]

    use super::{GAVE_UP_ACTION, give_up};
    use crate::brief_retry::{Phase, Record, publish, render};

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(format!("/tmp/ae-leg.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a scratch dir");
        dir
    }

    fn record() -> Record {
        Record {
            slot: "spawned.1".to_owned(),
            reference: "spawn-spawned.1".to_owned(),
            pane: "%105".to_owned(),
            launch_id: "tok-1".to_owned(),
            actor: "lead".to_owned(),
            attempts: 1,
            created: 1_789_100_000,
            phase: Phase::Armed,
            body: "⟦ae:brief from lead⟧\nbuild it".to_owned(),
        }
    }

    /// The WHOLE event line, not two fields of it: the ledger must name the
    /// ORIGINAL spawner as the actor, the seat as the target and the spawn's
    /// own reference — the same authority the pane's provenance line carries.
    /// Pinning the full line is what locks the shared event builder's output,
    /// so a change there cannot quietly reshape a brief's audit trail.
    #[test]
    fn a_give_up_names_the_spawner_the_seat_and_the_spawns_own_reference() {
        let dir = scratch("gaveup");
        let record = record();
        let witness = render(&record);
        assert!(publish(&dir, &record).is_ok(), "a record to give up");

        let mut err = Vec::new();
        assert!(
            give_up(
                &dir,
                &record,
                "scribe",
                "delivery was attempted twice",
                witness.as_bytes(),
                crate::time::Timestamp::from_epoch(1_789_100_600),
                &mut err,
            )
            .is_ok(),
            "the give-up should write"
        );

        let line = std::fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default();
        // THE WHOLE LINE, not a field of it. The shared event builder renders
        // this, so pinning every key and its order is what makes a change there
        // fail HERE rather than quietly reshape a brief's audit trail. Only the
        // clock is a substitution.
        let expected = format!(
            "{{\"ts\":\"2026-09-11T04:23:20Z\",\"actor\":\"lead\",\"action\":\"{GAVE_UP_ACTION}\",\"target\":\"scribe\",\"ref\":\"spawn-spawned.1\",\"target_slot\":\"spawned.1\",\"summary\":\"delivery was attempted twice (attempts 1, age 600s); the brief is preserved at undelivered.scribe.txt\"}}"
        );
        assert_eq!(line.trim_end(), expected, "the give-up event line");
        // The record is gone, and nothing else was written to the ledger.
        assert!(
            !crate::brief_retry::path(&dir, "spawned.1").exists(),
            "a given-up record is removed"
        );
        assert_eq!(line.lines().count(), 1, "exactly one event: {line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE SILENCE IS DELIBERATE. When the compare-and-swap finds the record
    /// changed, a successor owns that name — a retire plus a re-spawn during
    /// the flight — so this flight owns nothing. It must not remove the
    /// successor's record and must not write a give-up event about a brief
    /// that is not the one it was carrying. It says so on stderr and stops.
    #[test]
    fn a_give_up_that_no_longer_owns_the_record_writes_nothing_to_the_ledger() {
        let dir = scratch("successor");
        let mine = record();
        let mut successor = record();
        successor.actor = "someone-else".to_owned();
        successor.created = 1_789_100_500;
        assert!(publish(&dir, &successor).is_ok(), "the successor's record");

        let mut err = Vec::new();
        assert!(
            give_up(
                &dir,
                &mine,
                "scribe",
                "delivery was attempted twice",
                render(&mine).as_bytes(),
                crate::time::Timestamp::from_epoch(1_789_100_600),
                &mut err,
            )
            .is_ok(),
            "a lost race is not an error"
        );

        assert!(
            std::fs::read_to_string(dir.join("events.jsonl")).is_err(),
            "no event may be written about a brief this flight no longer owns"
        );
        // The successor is untouched, bytes for bytes.
        assert_eq!(
            std::fs::read_to_string(crate::brief_retry::path(&dir, "spawned.1")).ok(),
            Some(render(&successor)),
            "the successor's record must survive intact"
        );
        assert!(
            String::from_utf8_lossy(&err).contains("was replaced before it could be given up"),
            "the lost race is still visible: {}",
            String::from_utf8_lossy(&err)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
