//! `ae reseat <session> <agent> --using <profile>`: move ONE seat to another
//! profile, in place.
//!
//! The pain, measured across this fleet: a seat whose vendor quota dies is lost
//! WITH ITS CONTEXT. The only way on was a fresh spawn under a new name plus a
//! handover written by hand, and the seat's slot, pane, name and history stayed
//! bound to a tool that can no longer answer.
//!
//! This moves the SAME seat: same slot, same pane, same name, same records. The
//! successor arrives on another tool and is handed ae's own account of the seat
//! — the seed pack `ae brief --seat` renders, the predecessor's last turns
//! included — as its first turn.
//!
//! IT KILLS NOTHING. The seat's tool must already be gone, and the proof is
//! [`crate::seat_relaunch::prove_dead`] — the same owner, asked with this
//! verb's word, so the two commands refuse for the same reasons in the same
//! order. Ending a live harness is a later slice.
//!
//! The order is the contract, and everything before the paste is undone by
//! doing nothing:
//!
//! 1. the argv, the session, the caller, the seat and the profile resolve —
//!    no state is written;
//! 2. under the session's lifecycle lock, the seat is proven dead;
//! 3. the seed pack is built (it reads the seat's recorded first message,
//!    which step 4 removes) and published as `seed.<agent>.md`;
//! 4. every per-slot launch file goes, so the successor's `_run` has no stale
//!    first message, no stale sid file and no start marker to resume from;
//! 5. the meta rows move in ONE guarded replacement;
//! 6. the pane line is pasted and the new tool observed;
//! 7. past the lock: the tool's own launch turn, then the seed.
//!
//! Two crash windows, both named and both recoverable by hand: between 4 and 5
//! the meta still names the OLD profile with no start marker, so `relaunch`
//! brings the seat back on the tool it had; between 5 and 6 the meta names the
//! NEW profile and the pane sits at its shell, which is exactly what
//! `relaunch` finishes.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::inventory::ServerId;
use crate::seat_relaunch::{Proven, RESEAT_VERB, Started, Target};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tool::ToolKind;
use crate::tracked::{self, EventFields};

/// The usage line.
pub const RESEAT_USAGE: &str = "Usage: ae reseat <session> <agent> --using <profile>";

/// The event action a reseat records, whatever its outcome.
const RESEAT_ACTION: &str = "reseat";

/// The argv, validated.
struct Parsed {
    session: String,
    agent: String,
    profile: String,
}

/// Parse `<session> <agent> --using <profile>`.
///
/// The flag may sit anywhere, as `spawn`'s does, because that is where a hand
/// puts it. Anything else beginning with `-` is refused rather than swallowed
/// as a name.
fn parse(tail: &[String]) -> Result<Parsed, String> {
    let mut names: Vec<&str> = Vec::new();
    let mut profile = String::new();
    let mut words = tail.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--using" => {
                let Some(value) = words.next() else {
                    return Err("Error: --using needs a profile name.".to_owned());
                };
                profile.clone_from(value);
            }
            flag if flag.starts_with("--using=") => {
                flag["--using=".len()..].clone_into(&mut profile);
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Error: unknown flag '{flag}'."));
            }
            name => names.push(name),
        }
    }
    let [session, agent] = names.as_slice() else {
        return Err(RESEAT_USAGE.to_owned());
    };
    if profile.is_empty() {
        return Err("Error: reseat needs --using <profile>.".to_owned());
    }
    // The same line a launch draws around its own override: `profile@client` is
    // a LAUNCH grammar, and this verb writes no client row — it removes the one
    // the launch left, because that override belongs to a profile the seat no
    // longer runs.
    if profile.contains('@') {
        return Err(format!(
            "Error: --using '{profile}' names a client override — profile@client is launch-only. \
             reseat takes a bare profile."
        ));
    }
    Ok(Parsed {
        session: (*session).to_owned(),
        agent: (*agent).to_owned(),
        profile,
    })
}

/// Where this seat's seed is published — beside the meta, 0600, and KEPT after
/// a successful move: it is what a human re-sends by hand when the turn did not
/// land, and what tells a later reader what the successor was told.
///
/// The name goes through the same sanitiser a slot does, for the same reason:
/// this is a FILENAME COMPONENT, and the agent it is built from is read back
/// out of a pane option and a hand-editable meta rather than re-validated
/// against the agent grammar here. ONE owner, so the path this publishes and
/// the path a refusal tells a human to `cat` cannot disagree.
fn seed_file(dir: &Path, agent: &str) -> PathBuf {
    dir.join(format!("seed.{}.md", crate::launch::safe_slot(agent)))
}

/// Who ran this, read ONCE: the caller rule, the self-reseat refusal and the
/// event's actor are three questions about the same pane.
#[derive(Default)]
struct Caller {
    /// `$TMUX_PANE`, when this ran inside tmux at all.
    pane: Option<String>,
    /// `@ae_agent` — the display ref a record names. Empty for a plain shell,
    /// which the record renders as `human`, exactly as `relaunch` does.
    display: String,
}

/// THE CALLER RULE. `Err` is the refusal, `Ok` the caller.
///
/// A pane ae STAMPED is a seat, and a seat may reseat only inside its own
/// session — the boundary `relaunch` draws, for the same reason. A plain shell
/// carries no stamp and may reseat any session, because the human is who this
/// verb is for: the moment a lead's own quota dies, no agent of that session
/// can run anything.
fn caller_of(session: &str) -> Result<Caller, String> {
    let pane = crate::doors::calling_pane_id();
    let Some((viewer, _)) = crate::actual_calling_pane() else {
        return Ok(Caller {
            pane,
            display: String::new(),
        });
    };
    let display = viewer.agent.clone().unwrap_or_default();
    let Some(slot) = viewer.slot.clone().filter(|slot| !slot.is_empty()) else {
        return Ok(Caller { pane, display });
    };
    match viewer.session.as_deref() {
        Some(name) if name == session => Ok(Caller { pane, display }),
        Some(name) => Err(format!(
            "Error: this pane is slot {slot} of '{name}', and reseat works on the caller's own \
             session only — run it from '{session}', or from a shell outside ae."
        )),
        None => Err(format!(
            "Error: this pane carries ae slot {slot} but tmux did not say which session it is in \
             — ae will not reseat '{session}' for a caller it cannot place."
        )),
    }
}

/// Resolve `agent` to a seat of `session`.
///
/// Deliberately NOT `relaunch`'s resolver: that one refuses every session but
/// the caller's, and this verb's caller may be a shell in none. The refusal
/// that matters here is [`caller_refusal`], already made.
fn resolve(dir: &Path, agent: &str, session: &str) -> Result<Target, String> {
    let (resolved, server): (tracked::Resolved, ServerId) =
        tracked::resolve_on(agent, session, dir)
            .map_err(|why| tracked::ResolveError::message(&why))?;
    if !resolved.session.is_empty() && resolved.session != session {
        return Err(format!(
            "Error: '{agent}' is on session '{}', not '{session}'.",
            resolved.session
        ));
    }
    Ok(Target {
        agent: if resolved.agent.is_empty() {
            agent.to_owned()
        } else {
            resolved.agent
        },
        slot: resolved.slot,
        pane: resolved.pane,
        server,
    })
}

/// The new profile resolved to the two facts the meta records.
///
/// The SAME resolution `crate::run::read_seat` makes for a seat that records no
/// client override, so a profile this verb accepts is one `_run` can launch —
/// asked BEFORE anything is written, because a profile that does not lex is a
/// refusal and not a half-moved seat.
fn resolve_profile(dir: &Path, bytes: &[u8], profile: &str, agent: &str) -> Result<Moving, String> {
    let value = |key: &str| crate::lifecycle::meta_value(bytes, key);
    let global = value("config");
    let local = crate::config::local_overlay(dir, &value("origin"));
    // The orchestrator seat's own overlay is not a profile source, exactly as
    // `read_seat` has it: reading it here would offer a profile the launch
    // would then refuse.
    let orchestrator_seat = local.as_deref().is_some_and(|path| {
        dir.parent()
            .and_then(Path::parent)
            .is_some_and(|home| crate::orchestrator::is_seat_overlay(path, home))
    });
    let cfg = crate::config::read_identity(
        (!global.is_empty()).then(|| Path::new(&global)),
        (!orchestrator_seat).then_some(local.as_deref()).flatten(),
    )
    .map_err(|why| why.to_string())?;
    let home = crate::doors::home();
    let command = cfg
        .command(profile, home.as_deref())
        .map_err(|why| why.to_string())?;
    let Some(command) = command.filter(|command| !command.as_str().trim().is_empty()) else {
        return Err(format!(
            "profile '{profile}' is not configured on this machine — '{agent}' cannot be moved to it"
        ));
    };
    let parsed = crate::launch_cmd::lex_simple_command(command.as_str())
        .map_err(|why| format!("profile '{profile}' is not one simple command — {why}"))?;
    // NO tool-class judgement here, deliberately: a seat may be moved to
    // exactly the profiles a LAUNCH accepts, and the launch asks the adapter
    // rows for its behaviour rather than refusing a binary it does not know.
    Ok(Moving {
        tool: parsed.tool(),
        binary: parsed.binary,
    })
}

/// The new profile's two recorded facts.
struct Moving {
    tool: ToolKind,
    binary: String,
}

/// `ae reseat <session> <agent> --using <profile>`.
///
/// # Errors
///
/// Only a failure to write `out` or `err`. Every refusal is an exit code.
#[allow(clippy::too_many_lines, reason = "the frozen order, kept in one place")]
pub(crate) fn run(
    root: &Path,
    world: Option<&crate::listing::World>,
    tail: &[String],
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let parsed = match parse(tail) {
        Ok(parsed) => parsed,
        Err(line) => {
            writeln!(err, "{line}")?;
            if line != RESEAT_USAGE {
                writeln!(err, "{RESEAT_USAGE}")?;
            }
            return Ok(EXIT_USAGE);
        }
    };
    // The seed is built from the world the caller already enumerated, so the
    // pack a successor is handed is the one `ae brief --seat` prints this
    // instant — and an incomplete enumeration refuses rather than packs a
    // session it only half saw.
    let Some(entry) = world.and_then(|world| {
        world
            .sessions
            .iter()
            .find(|entry| entry.name == parsed.session)
    }) else {
        writeln!(
            err,
            "Error: no session named '{}' that ae could read.",
            parsed.session
        )?;
        return Ok(EXIT_FAILED);
    };
    let dir = crate::inventory::Roots::under(root)
        .sessions()
        .join(&parsed.session);
    let caller = match caller_of(&parsed.session) {
        Ok(caller) => caller,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_FAILED);
        }
    };
    // THE ROSTER FIRST, and tmux only after it. Everything a typo gets wrong —
    // the seat's name, the profile — is a DURABLE fact, and answering it from
    // the meta means a stopped session diagnoses the argv exactly as a running
    // one does instead of refusing for want of a pane.
    let bytes = crate::meta::read_bytes(&dir).unwrap_or_default();
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let Some(seated) = meta.roster().iter().find(|row| row.name == parsed.agent) else {
        let roster: Vec<&str> = meta.roster().iter().map(|row| row.name.as_str()).collect();
        writeln!(
            err,
            "Error: '{}' has no seat named '{}'; its roster is {}.",
            parsed.session,
            parsed.agent,
            if roster.is_empty() {
                "empty".to_owned()
            } else {
                roster.join(", ")
            }
        )?;
        return Ok(EXIT_FAILED);
    };
    let (slot, recorded) = (
        seated.slot.clone(),
        seated.profile.clone().unwrap_or_default(),
    );
    // What the seat recorded BEFORE the move, for the two messages that must
    // say what a half-finished move left behind. A seat with no profile row is
    // legal input: the move is what gives it one, and an empty string never
    // equals the non-empty profile the parse guarantees.
    let was = if recorded.is_empty() {
        "no profile".to_owned()
    } else {
        format!("profile '{recorded}'")
    };
    if recorded == parsed.profile {
        writeln!(
            err,
            "Error: '{}' already runs profile '{}' — there is nothing to move. `relaunch {}` \
             brings it back on the same profile.",
            parsed.agent, parsed.profile, parsed.agent
        )?;
        return Ok(EXIT_FAILED);
    }
    let moving = match resolve_profile(&dir, &bytes, &parsed.profile, &parsed.agent) {
        Ok(moving) => moving,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was reseated.")?;
            return Ok(EXIT_FAILED);
        }
    };
    let target = match resolve(&dir, &parsed.agent, &parsed.session) {
        Ok(target) => target,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_FAILED);
        }
    };
    // NOT YOUR OWN SEAT. The dead proof would refuse a live caller anyway, so
    // this closes no hole today — it states the rule instead of leaving it to
    // an accident of ordering, and it says the useful thing: the seat running
    // this command cannot be the seat whose tool is replaced under it.
    if caller.pane.as_deref() == Some(target.pane.as_str()) {
        writeln!(
            err,
            "Error: '{}' is THIS pane — a seat cannot reseat itself, because the tool running the \
             command is the one that would be replaced. Ask another seat, or run it from a shell.",
            target.agent
        )?;
        return Ok(EXIT_FAILED);
    }
    // The pane's own stamp against the roster. They disagree only when a stamp
    // is stale or a meta was hand-edited, and the move would then be published
    // for one seat and pasted into another's pane.
    if target.slot != slot {
        writeln!(
            err,
            "Error: pane {} is stamped slot {} but '{}' seats '{}' at {slot} — fix the meta, or \
             stop and resume the session.",
            target.pane, target.slot, parsed.session, parsed.agent
        )?;
        return Ok(EXIT_FAILED);
    }

    // THE LOCK, taken the way every lifecycle writer takes it: the dead proof,
    // the seed, the slot cleanup, the meta move and the paste are ONE
    // transaction, so no stop, compact, relaunch or second reseat interleaves.
    let Ok(lifecycle) = crate::lifecycle::lock(root, &parsed.session) else {
        writeln!(
            err,
            "Error: another lifecycle operation holds '{}' — it waited and gave up. Try again \
             once that finishes.",
            parsed.session
        )?;
        return Ok(EXIT_FAILED);
    };
    let Some(before) = crate::seat_relaunch::prove_dead(&dir, &target, RESEAT_VERB, err)? else {
        return Ok(EXIT_FAILED);
    };
    // BUILT FIRST, because the pack reads the seat's recorded first message and
    // the cleanup below removes that file.
    let seed = match crate::seat_pack(
        entry,
        &dir,
        crate::doors::home().as_deref(),
        now,
        &target.agent,
    ) {
        Ok(pack) => crate::provenance::first_line(&crate::provenance::ctx(), &pack),
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was reseated.")?;
            return Ok(EXIT_FAILED);
        }
    };
    if let Err(why) = crate::launch::publish_data(&seed_file(&dir, &target.agent), seed.as_bytes())
    {
        writeln!(err, "Error: {why}. Nothing was reseated.")?;
        return Ok(EXIT_FAILED);
    }
    if let Err(why) = crate::run::clear_slot(&dir, &target.slot) {
        writeln!(
            err,
            "Error: the launch files of slot {} could not be cleared ({why}) — nothing else was \
             touched and '{}' still records {was}.",
            target.slot, target.agent
        )?;
        return Ok(EXIT_FAILED);
    }
    let conversation = if crate::launch::takes_launch_session_id(moving.tool) {
        crate::launch::generate_uuid()
    } else {
        crate::launch::PENDING.to_owned()
    };
    let moved = crate::meta::publish_seat_move(
        &dir,
        &target.slot,
        &target.agent,
        &crate::meta::SeatMove {
            profile: &parsed.profile,
            binary: &moving.binary,
            harness_session: &conversation,
            launch_id: &crate::session_launch::launch_token(moving.tool, None),
            capture_floor: now.epoch(),
        },
    );
    if let Err(why) = moved {
        writeln!(
            err,
            "Error: the seat move could not be recorded ({}) — slot {} has no launch files now \
             and still records {was}. Re-run the reseat.",
            why.cause(),
            target.slot
        )?;
        return Ok(EXIT_FAILED);
    }
    // Re-read: the command, the tool and the conversation are the NEW profile's
    // now, and `start` pastes what `read_seat` composes from the moved rows.
    let after = match crate::run::read_seat(&dir, &target.slot, None) {
        Ok(seat) => Proven {
            seat,
            agent_bin: moving.binary.clone(),
            work_dir: before.work_dir,
            id_before: conversation,
        },
        Err(why) => {
            writeln!(
                err,
                "Error: {why}. '{}' already records profile '{}' and its pane is at a shell, so \
                 `relaunch {}` finishes the move once the profile reads.",
                target.agent, parsed.profile, target.agent
            )?;
            return Ok(EXIT_FAILED);
        }
    };
    let at = Record {
        caller: &caller,
        now,
        from: &recorded,
        to: &parsed.profile,
        // The conversation the PREDECESSOR held, read off the row the move
        // wrote: `reseated` appends it only when it can prove it, so an empty
        // one here is the same "nothing to hand on" the roster records.
        prior: &before.id_before,
    };
    let started = crate::seat_relaunch::start(&dir, &target, &after, now, RESEAT_VERB, err)?;
    // PAST THE LOCK before any readiness wait: a gated turn blocks up to 45s,
    // and nothing else may be held out of the session's lifecycle for that.
    drop(lifecycle);
    match started {
        // Nothing reached the pane and the refusing site already said why. The
        // meta HAS moved, so the next step is named rather than left implicit.
        Started::NotPasted => {
            writeln!(
                err,
                "  '{}' records profile '{}' now — `relaunch {}` starts it.",
                target.agent, parsed.profile, target.agent
            )?;
            Ok(EXIT_FAILED)
        }
        Started::NotSeen => {
            record(&dir, &at, &target, "pasted, tool not seen");
            writeln!(
                err,
                "Error: '{}' moved to '{}' and its tool was not seen: look at the pane; \
                 `relaunch {}` finishes the move once it is back at a shell.",
                target.agent, parsed.profile, target.agent
            )?;
            Ok(EXIT_FAILED)
        }
        Started::Running { resuming_seat } => finish(
            &dir,
            &target,
            &after,
            resuming_seat,
            &parsed,
            &seed,
            &at,
            out,
            err,
        ),
    }
}

/// Everything past the lock: the tool's own launch turn, the seed turn, the
/// post-launch capture, the last identity check and the exit rule.
#[allow(
    clippy::too_many_arguments,
    reason = "the frozen helper's inputs, spelled out rather than bundled"
)]
fn finish(
    dir: &Path,
    target: &Target,
    proven: &Proven,
    resuming_seat: bool,
    parsed: &Parsed,
    seed: &str,
    at: &Record<'_>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let tool = proven.seat.tool;
    // The tool's OWN launch turn first, where its adapter has one: codex's
    // rollout does not exist until a user turn, and the registration handshake
    // it carries must not be buried under a 24 KB seed.
    let prompt = crate::launch::initial_prompt_for(tool, dir, &target.slot);
    let launch_turn = if prompt.is_empty()
        || !crate::session_launch::launch_turn_is_pasted(tool, resuming_seat)
    {
        None
    } else {
        Some(crate::session_launch::deliver_launch_turn(
            dir,
            &target.server,
            &target.slot,
            &target.pane,
            tool,
            &prompt,
            err,
        )?)
    };
    // THE SEED, as ae's own setup turn: `deliver_launch_turn` pastes its text
    // verbatim, so the marker is put on here by the ONE owner that spells it.
    let seed_turn = crate::session_launch::deliver_launch_turn(
        dir,
        &target.server,
        &target.slot,
        &target.pane,
        tool,
        seed,
        err,
    )?;
    // Only a slot that reads `pending` NOW needs capturing: `_run`'s own
    // in-pane fallback may have cleared it after the move.
    let id_after = crate::lifecycle::meta_value(
        &crate::meta::read_bytes(dir).unwrap_or_default(),
        &format!("harness_session.{}", target.slot),
    );
    if tool.adapter().capture.is_needed() && id_after == crate::launch::PENDING {
        crate::session_launch::capture::start(
            dir,
            &[crate::session_launch::capture::Target {
                slot: target.slot.clone(),
                tool,
                pane: target.pane.clone(),
            }],
        );
    }
    // LAST: the new tool may have died while the turns were delivered, and
    // exit 0 means the seat is up NOW.
    if !crate::seat_relaunch::observe_identity(target, &proven.agent_bin, true) {
        record(dir, at, target, "tool stopped after the reseat");
        writeln!(
            err,
            "Error: '{}' started on '{}' and is gone again (pane {}) — look at the pane.",
            target.agent, parsed.profile, target.pane
        )?;
        return Ok(EXIT_FAILED);
    }
    let (launch_ok, launch_word) =
        launch_turn.map_or((true, ""), crate::seat_relaunch::turn_verdict);
    let (seed_ok, seed_word) = crate::seat_relaunch::turn_verdict(seed_turn);
    let launch_note = if launch_word.is_empty() {
        String::new()
    } else {
        format!(", launch turn {launch_word}")
    };
    let line = format!(
        "reseated {} (pane {}, slot {}) to {}{launch_note}, seed {seed_word}",
        target.agent, target.pane, target.slot, parsed.profile
    );
    record(dir, at, target, &line);
    if !launch_ok || !seed_ok {
        writeln!(err, "Error: {line} — a turn never landed.")?;
        writeln!(
            err,
            "  the seat is up; send its seed by hand: {}/send {} \"$(cat {})\"",
            dir.display(),
            target.agent,
            seed_file(dir, &target.agent).display()
        )?;
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "{line}")?;
    Ok(0)
}

/// What every `reseat` record carries besides its outcome: WHO asked, and the
/// move itself.
///
/// The move is on the record because the meta only keeps the CURRENT profile:
/// a later reader asking what this seat used to run has the predecessor's id
/// in the roster but nothing that names the profile it ran under, and the
/// event is the only place that pairing can live.
struct Record<'a> {
    caller: &'a Caller,
    now: Timestamp,
    from: &'a str,
    to: &'a str,
    prior: &'a str,
}

/// One `reseat` record for every attempt that REACHED the pane — the seat's
/// history is the only place a later reader can see that its TOOL changed, and
/// the only place that must not claim a paste that never was.
/// The event's own sentence. A row ae cannot prove reads `none`, and that
/// includes [`crate::launch::PENDING`]: it is the word for a conversation that
/// never resolved, `reseated` hands on no prior row for it, and an audit line
/// must not read as if one were being left behind.
fn summary(outcome: &str, from: &str, to: &str, prior: &str) -> String {
    let named = |value: &str| {
        if value.is_empty() || value == crate::launch::PENDING {
            "none".to_owned()
        } else {
            value.to_owned()
        }
    };
    format!(
        "{outcome} [from {} to {to}, prior {}]",
        named(from),
        named(prior)
    )
}

fn record(dir: &Path, at: &Record<'_>, target: &Target, outcome: &str) {
    let summary = summary(outcome, at.from, at.to, at.prior);
    let _ = crate::store::open(dir).append_event(&tracked::event_line(&EventFields {
        ts: at.now,
        // The caller's own display ref, exactly as `relaunch` records it: an
        // agent that moves a peer's seat must not appear in the audit trail as
        // the human, and a plain shell IS the human.
        actor: if at.caller.display.is_empty() {
            "human"
        } else {
            &at.caller.display
        },
        action: RESEAT_ACTION,
        target: &target.agent,
        reference: "",
        actor_slot: "",
        actor_session: "",
        target_slot: &target.slot,
        target_session: "",
        target_server: "",
        target_pane: &target.pane,
        target_session_uuid: "",
        caller_server: "",
        caller_pane: at.caller.pane.as_deref().unwrap_or_default(),
        caller_session_uuid: "",
        identity_gap: "",
        summary: &summary,
        body_file: "",
    }));
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn parsed(words: &[&str]) -> Result<(String, String, String), String> {
        super::parse(&argv(words)).map(|parsed| (parsed.session, parsed.agent, parsed.profile))
    }

    #[test]
    fn a_conversation_that_never_resolved_is_nothing_to_hand_on() {
        // The seat moved, and what it leaves behind is a word rather than an
        // id. The meta is already right — `reseated` writes no prior row it
        // cannot prove — and the audit line has to say the same thing.
        assert_eq!(
            super::summary("reseated", "a", "b", crate::launch::PENDING),
            "reseated [from a to b, prior none]"
        );
        assert_eq!(
            super::summary("reseated", "", "b", "11111111-1111-4111-8111-111111111111"),
            "reseated [from none to b, prior 11111111-1111-4111-8111-111111111111]"
        );
    }

    #[test]
    fn the_flag_sits_anywhere_and_every_other_dash_word_is_refused() {
        let want = Ok(("work".to_owned(), "lead".to_owned(), "lunam".to_owned()));
        // Where a hand puts it: after the names, between them, in front, and
        // in the `=` spelling a shell completion produces.
        for words in [
            &["work", "lead", "--using", "lunam"][..],
            &["work", "--using", "lunam", "lead"][..],
            &["--using", "lunam", "work", "lead"][..],
            &["work", "lead", "--using=lunam"][..],
        ] {
            assert_eq!(parsed(words), want, "{words:?}");
        }
        // A dash word is never a name: swallowing one would reseat a seat the
        // caller did not type.
        assert_eq!(
            parsed(&["work", "-f", "lead", "--using", "lunam"]),
            Err("Error: unknown flag '-f'.".to_owned())
        );
    }

    #[test]
    fn an_incomplete_argv_names_what_is_missing() {
        assert_eq!(parsed(&[]), Err(super::RESEAT_USAGE.to_owned()));
        assert_eq!(parsed(&["work"]), Err(super::RESEAT_USAGE.to_owned()));
        assert_eq!(
            parsed(&["work", "lead", "extra", "--using", "lunam"]),
            Err(super::RESEAT_USAGE.to_owned())
        );
        assert_eq!(
            parsed(&["work", "lead"]),
            Err("Error: reseat needs --using <profile>.".to_owned())
        );
        assert_eq!(
            parsed(&["work", "lead", "--using"]),
            Err("Error: --using needs a profile name.".to_owned())
        );
    }

    #[test]
    fn a_client_override_is_refused_because_this_verb_writes_no_client_row() {
        // The launch grammar `profile@client` pins a seat to one [clients]
        // label. A reseat REMOVES that row, so accepting the spelling here
        // would silently drop the half the caller typed.
        let refusal = parsed(&["work", "lead", "--using", "lunam@cc-mic"])
            .expect_err("a client override is refused");
        assert!(refusal.contains("launch-only"), "{refusal}");
        assert!(refusal.contains("lunam@cc-mic"), "{refusal}");
    }

    #[test]
    fn the_seed_is_published_beside_the_meta_under_the_seats_own_name() {
        // Named for the SEAT, not the slot: it is what a human `cat`s when a
        // turn did not land, and the refusal that says so prints this path.
        assert_eq!(
            super::seed_file(Path::new("/s/work"), "colead"),
            Path::new("/s/work/seed.colead.md")
        );
        // A name that is not a filename component cannot escape the session
        // directory, whatever put it in the meta or the pane option. The dot
        // SURVIVES, because a slot is `spawned.0` and the sanitiser is shared;
        // what cannot survive is the separator, so `..` is inert here.
        let escaped = super::seed_file(Path::new("/s/work"), "../../etc/x");
        assert_eq!(escaped, Path::new("/s/work/seed..._.._etc_x.md"));
        assert_eq!(escaped.parent(), Some(Path::new("/s/work")));
    }
}
