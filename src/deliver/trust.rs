//! The ONE answerer of a measured folder-trust modal: at most one arrow and
//! one Enter per tool start, under the pane's send-lock.
//!
//! [`crate::autotrust`] proves the seat and keeps the records; this is the
//! only place a key is sent for it. The order is the contract: two equal
//! reads, a proof, a durable reservation read back, then — only if trust-yes
//! is not selected — one arrow and two more equal reads proving trust-yes,
//! then Enter. Every key re-proves first and is recorded before the next.

use std::io::Write;
use std::time::Duration;

use crate::autotrust::{self, Proof, Seat, fail};
use crate::tmux::Key;
use crate::watchdog::TrustSighting;

/// The gap between the two reads a frame must survive, here and in `reseat`'s
/// stop. Between a key and its redraw, or between Enter and a drawn spinner,
/// the frame is in flight, and one read of it is not proof.
pub(crate) const FRAME_GAP: Duration = Duration::from_secs(1);

/// Reads after the Enter before the modal counts as still drawn.
const GONE_READS: usize = 3;

/// What one attempt did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TrustAnswer {
    /// The modal is gone after the Enter; the recorded summary.
    Trusted(String),
    /// Nothing sent, nothing spent: a later attempt may answer.
    NotYet(String),
    /// Nothing sent: this start is not ae's to answer, or is already spent.
    Declined(String),
    /// The start is spent and the modal was not answered.
    Failed(String),
}

/// Answer the trust modal on `seat`'s pane. `err` receives the lines a record
/// could not carry.
pub(crate) fn answer_trust(seat: &Seat<'_>, err: &mut impl Write) -> TrustAnswer {
    let Some(_held) = super::lock_target(seat.dir, seat.pane, Duration::ZERO) else {
        return TrustAnswer::NotYet("another delivery holds the pane".to_owned());
    };
    let first = match steady(seat, false) {
        Ok(sighting) => sighting,
        Err(why) => return TrustAnswer::NotYet(why),
    };
    let proof = match autotrust::prove(seat, &first.folder) {
        Ok(proof) => proof,
        Err(why) => return TrustAnswer::Declined(why),
    };
    if let Some(why) = viewer(seat) {
        return TrustAnswer::NotYet(why);
    }
    if let Err(why) = autotrust::reserve(seat, &proof, first.toward_yes.is_none()) {
        return TrustAnswer::Declined(why);
    }
    if let Some(arrow) = first.toward_yes {
        if let Err(stop) = press(seat, &proof, &first.folder, arrow, err) {
            return stop;
        }
        match steady(seat, true) {
            Ok(after) if after.toward_yes.is_none() && after.folder == first.folder => {}
            Ok(_) => {
                return fail(
                    seat,
                    &proof,
                    ("after-nav", "trust-yes is not selected"),
                    err,
                );
            }
            Err(why) => return fail(seat, &proof, ("after-nav", &why), err),
        }
    }
    if let Err(stop) = press(seat, &proof, &first.folder, Key::Enter, err) {
        return stop;
    }
    for _ in 0..GONE_READS {
        std::thread::sleep(FRAME_GAP);
        if matches!(read(seat), Frame::Clear) {
            return autotrust::trusted(seat, &proof, err)
                .map_or_else(TrustAnswer::Failed, TrustAnswer::Trusted);
        }
    }
    fail(
        seat,
        &proof,
        ("after-enter", "the trust modal is still drawn"),
        err,
    )
}

/// Re-prove, press `key`, and record it before anything else is pressed.
fn press(
    seat: &Seat<'_>,
    proof: &Proof,
    folder: &str,
    key: Key,
    err: &mut impl Write,
) -> Result<(), TrustAnswer> {
    let (step, name) = match key {
        Key::Up => ("nav", "Up"),
        Key::Down => ("nav", "Down"),
        Key::Enter => ("enter", "Enter"),
        Key::ClearLine | Key::LineEnd | Key::Escape | Key::CancelCopyMode => {
            return Err(fail(seat, proof, ("nav", "not a trust key"), err));
        }
    };
    match autotrust::prove(seat, folder) {
        Ok(again) if again == *proof => {}
        Ok(_) => return Err(fail(seat, proof, (step, "the seat changed"), err)),
        Err(why) => return Err(fail(seat, proof, (step, &why), err)),
    }
    // Past the reservation a held key cannot wait for a later cycle: the
    // start is spent, so the stop is recorded and the prompt named.
    if let Some(why) = viewer(seat) {
        return Err(fail(seat, proof, (step, &why), err));
    }
    let pressed = crate::transport::send_key(seat.server, seat.pane, key);
    let word = if pressed { "sent" } else { "refused" };
    if let Err(why) = autotrust::record(seat, proof, autotrust::KEY, &format!("{name} {word}")) {
        return Err(fail(seat, proof, ("journal", &why.to_string()), err));
    }
    if pressed {
        Ok(())
    } else {
        Err(fail(
            seat,
            proof,
            (step, &format!("tmux refused {name}")),
            err,
        ))
    }
}

/// Why a human may be at the pane: a client viewing it gave input inside
/// [`super::VIEW_GRACE`], and no key of ae's may race theirs.
fn viewer(seat: &Seat<'_>) -> Option<String> {
    held(super::recently_viewed(seat.server, seat.pane))
}

fn held(age: Option<i64>) -> Option<String> {
    age.map(|age| format!("a client viewing the pane gave input {age}s ago"))
}

/// Two reads `FRAME_GAP` apart showing the SAME trust block. `settle` waits
/// one gap first, for a frame a key has just moved.
fn steady(seat: &Seat<'_>, settle: bool) -> Result<TrustSighting, String> {
    if settle {
        std::thread::sleep(FRAME_GAP);
    }
    let Frame::Trust(first) = read(seat) else {
        return Err("no trust modal is drawn".to_owned());
    };
    std::thread::sleep(FRAME_GAP);
    let Frame::Trust(second) = read(seat) else {
        return Err("no trust modal is drawn".to_owned());
    };
    if first == second {
        Ok(second)
    } else {
        Err("the trust modal moved between two reads".to_owned())
    }
}

/// One capture, as the answer reads it.
enum Frame {
    /// No prompt only the human may answer is drawn.
    Clear,
    /// The measured trust block.
    Trust(TrustSighting),
    /// Another prompt, or a failed capture — proof of nothing, gone included.
    Neither,
}

fn read(seat: &Seat<'_>) -> Frame {
    let Some(frame) = crate::transport::capture_pane(seat.server, seat.pane) else {
        return Frame::Neither;
    };
    match crate::watchdog::human_prompt_class(&frame, seat.tool) {
        None => Frame::Clear,
        Some(found) => found.trust.map_or(Frame::Neither, Frame::Trust),
    }
}

#[cfg(test)]
mod tests {
    /// Plan E11: client input inside `VIEW_GRACE` sends no key. The gate is
    /// asked under the send-lock before the reservation (a hold spends
    /// nothing) and again before every key, ahead of the send.
    #[test]
    fn client_input_inside_view_grace_holds_every_trust_key() {
        assert_eq!(super::held(None), None);
        let why = super::held(Some(0)).expect("input just now holds");
        assert!(why.contains("gave input 0s ago"), "{why}");
        let source = include_str!("trust.rs");
        let body = |from: &str, to: &str| {
            let start = source.find(from).expect(from);
            let rest = &source[start..];
            &rest[..rest.find(to).expect(to)]
        };
        let at = |body: &str, token: &str| body.find(token).expect(token);
        let answer = body("pub(crate) fn answer_trust(", "\nfn press(");
        assert!(at(answer, "lock_target(") < at(answer, "autotrust::prove("));
        assert!(at(answer, "autotrust::prove(") < at(answer, "viewer(seat)"));
        assert!(at(answer, "viewer(seat)") < at(answer, "autotrust::reserve("));
        let press = body("\nfn press(", "\n/// Why a human may be at the pane");
        assert!(at(press, "viewer(seat)") < at(press, "send_key("));
    }
}
