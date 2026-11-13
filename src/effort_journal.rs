//! The effort JOURNAL vocabulary: the watchdog's `effort-*` actions, why a
//! record was discarded, the one journal line ae writes for a seat, and
//! whether the journal already ends an episode. It reads no world and names
//! nothing above the state layer, so the meta writer can journal a reseat's
//! or a retire's discard; [`crate::effort`] re-exports it and owns the fold.

use crate::events::{Event, SessionKey};
use crate::time::Timestamp;

/// An episode opens: ref `<ep>`, summary `threshold <at>`. `<ep>` is the
/// record's own epoch.
pub const EPISODE: &str = "effort-episode";
/// A move is reserved: ref `<ep>-<down|up>`, summary `<from>-><to>`.
pub const RESERVED: &str = "effort-reserved";
/// The step down is proven: ref `<ep>-down`, summary `<from>-><to>`.
pub const APPLIED: &str = "effort-applied";
/// The step back up is proven, closing the episode: ref `<ep>-up`.
pub const RESTORED: &str = "effort-restored";
/// A reserved move is proven to have failed: ref `<ep>-<down|up>`.
pub const FAILED: &str = "effort-failed";
/// A reason named once per episode: ref `<ep>`, summary the reason word.
pub const HELD: &str = "effort-held";
/// The human owns this episode now: ref `<ep>`, summary
/// `manual-effort` or `baseline-changed`.
pub const YIELDED: &str = "effort-yielded";
/// The episode ends unmoved or yielded: ref `<ep>`.
pub const REARMED: &str = "effort-rearmed";
/// A resume, a reseat or a retire dropped the seat's record and everything the
/// journal remembers of its episodes: ref `<ep>` or empty, summary `off`,
/// `fresh`, `reseat` or `retire`.
pub const DISCARDED: &str = "effort-discarded";

/// A canonical unsigned decimal: digits only, no leading zero.
pub(crate) fn decimal<T: std::str::FromStr>(text: &str) -> Option<T> {
    let canonical = text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.is_empty() && !text.starts_with('0'));
    canonical.then(|| text.parse().ok()).flatten()
}

/// A positive epoch in canonical decimal.
pub(crate) fn epoch(text: &str) -> Option<i64> {
    decimal(text).filter(|value: &i64| *value > 0)
}

/// Why a resume, a reseat or a retire drops a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discarded {
    /// `auto_effort = off`.
    Off,
    /// The seat does not resume the record's conversation.
    Fresh,
    /// A reseat moved the seat to another setup.
    Reseat,
    /// A retire removed the seat.
    Retire,
}

impl Discarded {
    /// The [`DISCARDED`] summary.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Fresh => "fresh",
            Self::Reseat => "reseat",
            Self::Retire => "retire",
        }
    }
}

/// The seat a [`DISCARDED`] record is routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscardSeat<'a> {
    /// The session name.
    pub session: &'a str,
    /// The meta `session_id`.
    pub session_id: &'a str,
    /// The roster slot.
    pub slot: &'a str,
    /// The agent name.
    pub agent: &'a str,
}

/// The ONE [`DISCARDED`] journal line, as the watchdog, routed to the seat by
/// slot and session id so a rename keeps it addressed. No trailing newline.
#[must_use]
pub fn discard_line(
    now: Timestamp,
    seat: &DiscardSeat<'_>,
    episode: Option<i64>,
    why: Discarded,
) -> String {
    let reference = episode.map(|key| key.to_string()).unwrap_or_default();
    line(now, seat, DISCARDED, &reference, why.word())
}

/// The ONE effort journal line, as the watchdog, routed to the seat by slot and
/// session id so a rename keeps it addressed. No trailing newline.
#[must_use]
pub fn line(
    now: Timestamp,
    seat: &DiscardSeat<'_>,
    action: &str,
    reference: &str,
    summary: &str,
) -> String {
    let line = crate::tracked::event_line(&crate::tracked::EventFields {
        ts: now,
        actor: crate::watchdog::WATCHDOG_ACTOR,
        action,
        target: seat.agent,
        reference,
        actor_slot: "",
        actor_session: "",
        target_slot: seat.slot,
        target_session: seat.session,
        actor_session_id: "",
        target_session_id: seat.session_id,
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary,
        body_file: "",
    });
    line.trim_end_matches('\n').to_owned()
}

/// Whether a watchdog [`DISCARDED`] record is one ae writes: ref empty or a
/// canonical epoch, summary exactly a [`Discarded`] word.
pub(crate) fn well_formed_discard(event: &Event) -> bool {
    let reference = event.reference.as_deref().unwrap_or_default();
    let summary = event.summary.as_deref().unwrap_or_default();
    (reference.is_empty() || epoch(reference).is_some())
        && [
            Discarded::Off,
            Discarded::Fresh,
            Discarded::Reseat,
            Discarded::Retire,
        ]
        .iter()
        .any(|why| why.word() == summary)
}

/// Whether the seat's journal already ends its CURRENT episode: a
/// well-formed watchdog [`DISCARDED`] whose ref is `episode` (empty for a
/// damaged record), addressed to the seat after the seat's last watchdog
/// [`EPISODE`], whatever its reason. An applied episode has ONE journaled
/// ending, whichever path wrote it: a reseat retried after a journaled
/// discard whose meta publish failed, or a reseat or a retire after another
/// path's discard whose publish failed, reuses that ending. Another actor,
/// slot or session, another ref, an unknown reason, or an ending before a
/// newer episode does not count; a record of another actor between them, such
/// as the reseat's own stop record, does not hide it.
#[must_use]
pub fn ended(
    events: &[Event],
    session: SessionKey<'_>,
    slot: &str,
    agent: &str,
    episode: Option<i64>,
) -> bool {
    let reference = episode.map(|key| key.to_string()).unwrap_or_default();
    let mut ended = false;
    for event in events {
        if event.actor != crate::watchdog::WATCHDOG_ACTOR
            || !crate::watchdog::event_is_addressed_to_in(event, session, slot, agent)
        {
            continue;
        }
        if event.action == EPISODE {
            ended = false;
        } else if event.action == DISCARDED
            && event.reference.as_deref().unwrap_or_default() == reference
            && well_formed_discard(event)
        {
            ended = true;
        }
    }
    ended
}
