//! A seat that died tells the agent that spawned it (#211).
//!
//! The watchdog already latches a seat's death and alerts the human; that path
//! is unchanged. This module owns what the spawner is told on top of it:
//! whom (the roster member the seat's newest `spawn` record names, never the
//! seat itself), what (one line naming the seat and the observed state), how
//! often (three tries, a retry only for a refusal the send helper proves came
//! before the paste) and how the journal proves it was told once per death
//! episode, across a daemon restart.
//!
//! ONE function reads the episode: [`episode`] folds the journal records of the
//! seat, and the daemon's own records of this cycle, into the episode's key
//! and whether the spawner was told in it. The key changes exactly at a
//! closer: a record the seat itself wrote (the routing rule every reader
//! shares, which never reads a record that speaks for the daemon as a seat's),
//! or a `dead-cleared`, `pane-back`, `spawn` or `reseat` addressed to it. A `dead-notice` is told for the
//! episode it follows. The durable half (told) and the in-memory half (the
//! [`Standing`] below) both come from that fold, so they cannot disagree.
//!
//! A death episode is the seat's whole stretch from its first sign of death to
//! the next closer, whichever way it dies: dropped to its shell, or its pane
//! gone, and either leg after the other. ONE [`Standing`] per seat
//! (`Carry::notices`) covers it, so a try the helper could not prove refused
//! is never repeated, and the three tries are spent once in all. The standing
//! remembers the key it was armed at: a closer since then, a positive reading
//! that the seat lives or the daemon's own clear of the latch starts the seat
//! over, and a closer that was already there never resets the budget again.
//!
//! The send helper books one `dead-notice` record per delivery (`ref` = the
//! dead seat's name, `target` = the spawner). Memory carries only the standing:
//! a restarted watchdog re-reads the journal and never repeats a told episode.
//! A `pane-back` this module writes when a told seat stands live again closes
//! the episode without retracting the human's pane-missing alert, which a
//! `dead-cleared` would.

use std::io::Write;

use super::{Acting, Cycle, Event, QuotaDelivery, read_events};
use crate::meta::RosterEntry;
use crate::procs::Descendancy;

/// The journal action of one delivered notice, as the send helper books it.
pub(super) const ACTION: &str = "dead-notice";
/// The record of a told seat that stands live again: no alert meaning.
pub(super) const BACK_ACTION: &str = "pane-back";
/// Tries one episode's notice gets, the first included.
const TRIES: u8 = 3;

/// How the spawner's seat was seen to die.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Death {
    /// The agent process exited and the pane fell back to its shell.
    Shell,
    /// The pane left the session.
    PaneGone,
}

impl Death {
    /// The observed state, in the words the notice quotes.
    const fn state(self) -> &'static str {
        match self {
            Self::Shell => "process dropped to shell",
            Self::PaneGone => "pane gone",
        }
    }
}

/// Where one seat's death-episode notice stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Notice {
    /// Nothing booked: no death seen, or the seat lives again.
    #[default]
    Idle,
    /// Owed to the spawner; `attempts` tries are spent.
    Owed { attempts: u8 },
    /// Delivered, judged not owed, or given up: nothing more this episode.
    Settled,
}

/// One seat's notice in this daemon's memory: where it stands, and the key of
/// the episode it stands for. Only [`Notice::Idle`] has no episode, and it
/// carries no key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Standing {
    notice: Notice,
    key: usize,
}

impl Standing {
    /// The standing at an episode key: a closer that landed since it was armed
    /// ended that episode, so the seat starts over.
    const fn at(self, key: usize) -> Self {
        if matches!(self.notice, Notice::Idle) || self.key == key {
            self
        } else {
            Self {
                notice: Notice::Idle,
                key: 0,
            }
        }
    }

    /// An idle standing armed at this key; every other one is kept.
    const fn armed(self, key: usize) -> Self {
        match self.notice {
            Notice::Idle => Self {
                notice: Notice::Owed { attempts: 0 },
                key,
            },
            _ => self,
        }
    }

    /// The same episode after a try.
    const fn then(self, notice: Notice) -> Self {
        Self { notice, ..self }
    }

    /// A standing owing the notice with `attempts` tries spent, at `key`.
    #[cfg(test)]
    pub(super) const fn owed(attempts: u8, key: usize) -> Self {
        Self {
            notice: Notice::Owed { attempts },
            key,
        }
    }

    /// A standing whose notice is over, at `key`.
    #[cfg(test)]
    pub(super) const fn settled(key: usize) -> Self {
        Self {
            notice: Notice::Settled,
            key,
        }
    }
}

/// What one cycle saw of a live pane's seat.
pub(super) struct Reading<'a> {
    /// The death latch before this cycle's booking.
    pub(super) was_latched: bool,
    /// The death latch after it.
    pub(super) latched: bool,
    /// The cycle booked its own `dead-cleared` for the seat: a record the
    /// events the cycle read at its start do not hold, and which a same-cycle
    /// re-latch hides behind both latch readings.
    pub(super) cleared: bool,
    /// The pane's foreground command.
    pub(super) command: &'a str,
}

/// The one line the spawner reads.
fn text(worker: &str, death: Death) -> String {
    format!("worker {worker} is DEAD — {}", death.state())
}

/// The roster member to tell: the actor of the newest `spawn` addressed to the
/// seat, when it still holds a seat of its own. A human, a console, a retired
/// or unknown spawner and the seat itself are nobody.
fn recipient<'a>(
    events: &'a [Event],
    roster: &'a [RosterEntry],
    session: &str,
    (slot, agent): (&str, &str),
) -> Option<&'a str> {
    let by = crate::autoreseat::spawner(events, session, slot, agent)?;
    roster
        .iter()
        .find(|entry| entry.name == by && entry.slot != slot)
        .map(|entry| entry.name.as_str())
}

/// One seat's death episode as the journal holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Episode {
    /// How many closers the seat has had: it changes exactly at one.
    key: usize,
    /// A `dead-notice` for the seat follows the newest closer.
    told: bool,
}

/// The seat's episode over `events`, the journal as it stands, the cycle's own
/// records included. The ONE rule of what ends an episode: the seat's own
/// record (the routing rule every reader shares: its key, or its display name
/// for a keyless legacy record, and never a record that speaks for the
/// daemon), or a retraction, return, spawn or reseat addressed to it. The
/// notice record is never a closer.
fn episode(events: &[Event], session: &str, (slot, agent): (&str, &str)) -> Episode {
    let mut seen = Episode::default();
    for event in events {
        if event.action == ACTION {
            seen.told |= event.reference.as_deref() == Some(agent);
        } else if closes(event, session, slot, agent) {
            seen = Episode {
                key: seen.key + 1,
                told: false,
            };
        }
    }
    seen
}

/// Whether the record ends a death episode of the seat.
fn closes(event: &Event, session: &str, slot: &str, agent: &str) -> bool {
    crate::watchdog::event_is_actor(event, session, slot, agent)
        || (crate::watchdog::event_is_addressed_to(event, session, slot, agent)
            && matches!(
                event.action.as_str(),
                "dead-cleared" | BACK_ACTION | "spawn" | crate::reseat::RESEAT_ACTION
            ))
}

/// Whether the journal holds a notice for the seat at all, which is the cheap
/// question that decides if its episode is worth folding.
fn notified(events: &[Event], agent: &str) -> bool {
    events
        .iter()
        .any(|event| event.action == ACTION && event.reference.as_deref() == Some(agent))
}

/// A POSITIVE reading that the seat lives: not dead, and either its harness is
/// seen under the pane or its foreground command is no shell. A probe gap on a
/// shell is no evidence of life.
fn alive(is_dead: bool, command: &str, descendancy: Descendancy) -> bool {
    !is_dead
        && (matches!(descendancy, Descendancy::Present)
            || !crate::watchdog::command_is_shell(command))
}

/// What one cycle's reading of the pane leaves of the seat's standing, before
/// any try, at episode key `key`. A closer the journal holds since the
/// standing was armed ends its episode first. A seat latched dead then arms an
/// idle standing and keeps every other; the daemon's own clear of the latch,
/// or a positive reading that the seat lives, ends the episode; any other
/// reading (a launch grace, a probe gap) leaves it as it was.
const fn after_reading(
    held: Standing,
    (was_latched, latched, lives): (bool, bool, bool),
    key: usize,
) -> Standing {
    let held = held.at(key);
    if latched {
        held.armed(key)
    } else if was_latched || lives {
        Standing {
            notice: Notice::Idle,
            key: 0,
        }
    } else {
        held
    }
}

/// What one try leaves: the new standing and, when the notice is lost, why.
fn after(delivery: QuotaDelivery, spent: u8) -> (Notice, Option<&'static str>) {
    match delivery {
        QuotaDelivery::Delivered => (Notice::Settled, None),
        QuotaDelivery::Retryable if spent + 1 < TRIES => (
            Notice::Owed {
                attempts: spent + 1,
            },
            None,
        ),
        QuotaDelivery::Retryable => (
            Notice::Settled,
            Some("refused before the paste on every try"),
        ),
        QuotaDelivery::Uncertain => (
            Notice::Settled,
            Some("not proven refused before the paste, so never retried"),
        ),
    }
}

/// The tries the standing has spent.
const fn spent(notice: Notice) -> u8 {
    match notice {
        Notice::Owed { attempts } => attempts,
        Notice::Idle | Notice::Settled => 0,
    }
}

impl Cycle<'_> {
    /// One pane's step of the death notice, after its effects are applied:
    /// the standing follows the reading and the journal, a seat latched dead is
    /// told (the wake settle holds the paste, and the owed standing keeps it
    /// for the first free cycle), and a told seat that stands live again books
    /// `pane-back`. `held` is (settling, the seat's standing).
    pub(super) fn notice_step(
        &self,
        on: &Acting<'_>,
        reading: &Reading<'_>,
        held: (bool, &mut Standing),
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let (settling, standing) = held;
        let lives = alive(on.seen.is_dead, reading.command, on.seen.descendancy);
        let seat = (on.slot, on.agent);
        // The records this cycle wrote for the seat are in the journal now.
        let fresh;
        let events = if reading.cleared {
            fresh = read_events(self.meta_dir);
            fresh.as_slice()
        } else {
            on.events
        };
        let back = lives && !reading.was_latched && notified(events, on.agent);
        let episode = if reading.latched || standing.notice != Notice::Idle || back {
            episode(events, self.session, seat)
        } else {
            Episode::default()
        };
        let latch = (reading.was_latched, reading.latched, lives);
        *standing = after_reading(*standing, latch, episode.key);
        if reading.latched {
            if !settling && matches!(standing.notice, Notice::Owed { .. }) {
                let tried = (Death::Shell, spent(standing.notice));
                let next = self.tell_spawner(seat, events, episode.told, tried, err)?;
                *standing = standing.then(next);
            }
            return Ok(());
        }
        // A latch the daemon itself clears books its own `dead-cleared`.
        if back && episode.told {
            self.emit(
                BACK_ACTION,
                on.agent,
                "seat live again — a new death is told again",
                err,
            )?;
        }
        Ok(())
    }

    /// Tell a missing roster seat's spawner on the seat's one standing: the
    /// human's pane-missing alert is untouched and is booked before this runs.
    /// `events` is the journal as of this sweep, which holds the cycle's own
    /// earlier records.
    pub(super) fn tell_missing(
        &self,
        entry: &RosterEntry,
        standing: &mut Standing,
        events: &[Event],
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let seat = (entry.slot.as_str(), entry.name.as_str());
        let episode = episode(events, self.session, seat);
        *standing = standing.at(episode.key).armed(episode.key);
        if standing.notice == Notice::Settled {
            return Ok(());
        }
        let tried = (Death::PaneGone, spent(standing.notice));
        let next = self.tell_spawner(seat, events, episode.told, tried, err)?;
        *standing = standing.then(next);
        Ok(())
    }

    /// One try for the seat: the standing it leaves. Nobody to tell, or an
    /// episode the journal holds as told, settles it without a paste.
    fn tell_spawner(
        &self,
        seat: (&str, &str),
        events: &[Event],
        told: bool,
        (death, tried): (Death, u8),
        err: &mut impl Write,
    ) -> crate::error::Result<Notice> {
        let Some(to) = recipient(events, &self.roster, self.session, seat) else {
            return Ok(Notice::Settled);
        };
        if told {
            return Ok(Notice::Settled);
        }
        let line = text(seat.1, death);
        let delivery = self.deliver(to, &line, ACTION, &line, Some(seat.1));
        let (next, lost) = after(super::quota_delivery(&delivery), tried);
        if let Some(why) = lost {
            writeln!(
                err,
                "ae: watchdog: {}: spawner {to} not told the seat is dead: {why}",
                seat.1
            )?;
        }
        Ok(next)
    }
}

#[cfg(test)]
#[path = "deadnotice_tests.rs"]
mod tests;
