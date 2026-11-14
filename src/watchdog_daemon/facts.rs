//! The fact helpers: what one window entry draws, the observed-cell rungs and the small readers the cycle shares.

use super::{
    AgentObservation, Descendancy, Event, Look, Mark, MissingState, Path, Published,
    RecordedClient, RosterEntry, SeatIdentity, SeatVerdict, Verdict, procs, store, theme, tmux,
};

/// A completed verdict cycle is a foreground-safe scheduling edge. The 200 ms
/// motion ticker never calls this; cadence prevents redundant later children.
pub(super) fn schedule_automatic_upgrade() {
    crate::autoupgrade::schedule();
}

/// The agents one window entry draws, in pane order.
///
/// Labels have already passed through [`theme::agent_label`]. A mark comes
/// first so a window's state is visible before its names: the mark itself
/// separates adjacent agents, so no brackets are needed. `working_frame`
/// replaces only working marks, allowing the ticker to animate the whole label
/// without re-reading a verdict.
pub(super) fn window_agents_line(
    agents: &[(String, Mark)],
    look: &Look,
    working_frame: Option<&theme::WorkingFrame>,
) -> String {
    agents
        .iter()
        .map(|(agent, mark)| {
            let (glyph, fg) = if *mark == Mark::Working {
                working_frame.map_or(
                    (mark.glyph(look.icons), look.palette.accent(*mark)),
                    |frame| (frame.glyph, frame.fg.as_str()),
                )
            } else {
                (mark.glyph(look.icons), look.palette.accent(*mark))
            };
            format!("#[fg={fg}]{glyph}#[default]{agent}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What ONE roster entry is saying.
///
/// The single owner of that judgement: the session's rolled-up attention and
/// therefore every other session's fleet strip read it here, so a slot whose
/// pane has gone missing cannot disappear from session attention.
pub(super) fn slot_mark(
    entry: &RosterEntry,
    by_slot: &[(String, SeatVerdict)],
    missing: &[(String, MissingState)],
) -> Mark {
    let found = by_slot.iter().find(|(slot, _)| *slot == entry.slot);
    found.map_or_else(
        || {
            let seen_absent = missing
                .iter()
                .any(|(slot, state)| *slot == entry.slot && state.streak > 0);
            if seen_absent {
                Mark::NeedsYou
            } else {
                Mark::Idle
            }
        },
        |(_, verdict)| verdict.mark(),
    )
}

/// How much of an entry's observed cells one attempt at the fact writes.
///
/// A roster that will not fit is degraded in RUNGS, and every rung is
/// fleet-wide: the model cell is a COLUMN, and one dropped for some rows and
/// not others is one the human cannot read down. The order spends the least
/// useful fact first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FactRung {
    /// Everything this cycle observed.
    Full,
    /// Models and clients, without the effort or the drift mark.
    NoEffort,
    /// The four v1 fields, the client and the task, with no observed model,
    /// effort or drift. The client and the task are the cheapest cells on the
    /// row and the ones that never drop.
    ClientOnly,
    /// Exactly today's v1 bytes.
    ///
    /// The LAST rung before nothing, and the reason it exists: v3 costs five
    /// more separators per entry, so without it a roster that fits today could
    /// vanish BECAUSE model cells were added — and a fact ae cannot publish is
    /// a roster the picker reports as unavailable.
    Legacy,
}

impl FactRung {
    /// The rungs, most informative first.
    const LADDER: [Self; 4] = [Self::Full, Self::NoEffort, Self::ClientOnly, Self::Legacy];

    /// The version word this rung announces.
    const fn version(self) -> &'static str {
        match self {
            Self::Full | Self::NoEffort | Self::ClientOnly => "v3",
            Self::Legacy => "v1",
        }
    }
}

/// What one roster seat's client cell shows: the operator's own `[clients]`
/// label, fully written.
///
/// Recorded truth first, current config second, nothing written to the meta:
/// the recorded `client.<slot>` override (the precedence `run::read_seat`
/// honors — the seat RUNS that client even when its row left the config, so
/// no lookup gates it), then the profile's `[clients]` row today, then the
/// recorded binary name, then today's short code. Every rung allowlists and
/// falls through; the answer is always emittable.
pub(super) fn seat_client_label(
    entry: &RosterEntry,
    cfg: Option<&crate::config::IdentityConfig>,
) -> String {
    if let RecordedClient::Label(label) = &entry.client
        && crate::config::is_client_label(label)
    {
        return label.clone();
    }
    if let (Some(cfg), Some(profile)) = (cfg, entry.profile.as_deref())
        && let Some(label) = cfg.profile_client_label(profile)
        && crate::config::is_client_label(&label)
    {
        return label;
    }
    if let Some(binary) = entry.binary.as_deref()
        && crate::config::is_client_label(binary)
    {
        return binary.to_owned();
    }
    crate::tool::ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default())
        .client_token()
        .to_owned()
}

/// The watchdog-owned agent fact in recorded roster order.
///
/// Present panes carry the verdict this cycle already computed. A roster seat
/// with no pane remains visible as `dead` with an empty navigation hint. Any
/// unrepresentable recorded identity rejects the whole value rather than
/// publishing a partial roster the picker could mistake for complete.
///
/// A seat's observed cells are its own: a model ae cannot represent — over
/// cap, or carrying a byte the grammar forbids — empties that ENTRY's trio and
/// nothing else ([`observed_cells`]). Only a whole fact that will not fit
/// walks the [`FactRung`] ladder.
pub(super) fn agents_fact(
    roster: &[RosterEntry],
    by_slot: &[AgentObservation],
    clients: &[(&str, String)],
    now_epoch: i64,
    interval_secs: u64,
) -> Option<String> {
    if roster.is_empty()
        || roster.len() > tmux::PICKER_AGENTS_MAX_COUNT
        || now_epoch < 0
        || !(1..=tmux::PICKER_AGENTS_MAX_INTERVAL_SECS).contains(&interval_secs)
    {
        return None;
    }
    FactRung::LADDER
        .into_iter()
        .find_map(|rung| fact_at(roster, by_slot, clients, now_epoch, interval_secs, rung))
}

/// One attempt at the fact, at exactly `rung`'s fidelity.
///
/// `None` means either a roster this writer must never publish at all — a
/// recorded identity it cannot spell — or simply one that does not fit at this
/// rung, which the ladder above answers by trying a plainer one.
#[allow(
    clippy::too_many_arguments,
    reason = "one composition of the cycle's already-derived roster slices"
)]
pub(super) fn fact_at(
    roster: &[RosterEntry],
    by_slot: &[AgentObservation],
    clients: &[(&str, String)],
    now_epoch: i64,
    interval_secs: u64,
    rung: FactRung,
) -> Option<String> {
    let mut value = format!("{};{now_epoch};{interval_secs}", rung.version());
    let mut names: Vec<&str> = Vec::new();
    for entry in roster {
        let profile = entry.profile.as_deref()?;
        if !crate::config::is_agent_name(&entry.name)
            || !crate::config::is_config_key(profile)
            || names.contains(&entry.name.as_str())
        {
            return None;
        }
        names.push(&entry.name);
        let found = by_slot.iter().find(|seen| seen.slot == entry.slot);
        let (state, pane) = found.map_or(("dead", ""), |seen| {
            (seen.verdict.reason(), seen.pane.as_str())
        });
        if !pane.is_empty() && !tmux::pane_id_is_valid(pane) {
            return None;
        }
        value.push(';');
        value.push_str(&entry.name);
        value.push(':');
        value.push_str(profile);
        value.push(':');
        value.push_str(state);
        value.push(':');
        value.push_str(pane);
        if rung != FactRung::Legacy {
            let client = clients
                .iter()
                .find(|(slot, _)| *slot == entry.slot)
                .map_or("-", |(_, client)| client.as_str());
            let (model, effort, drift) = observed_cells(found.map(|seen| &seen.identity), rung);
            value.push(':');
            value.push_str(client);
            value.push(':');
            value.push_str(model);
            value.push(':');
            value.push_str(effort);
            value.push(':');
            value.push_str(drift);
            // The cheapest cell, so every rung but Legacy keeps it: `1` when
            // the seat tool's job is proven, else empty.
            value.push(':');
            if found.is_some_and(|seen| seen.verdict.task) {
                value.push('1');
            }
        }
        if value.len() > tmux::PICKER_AGENTS_MAX_BYTES {
            return None;
        }
    }
    Some(value)
}

/// One seat's three observed cells at `rung`, empty where nothing may be said.
///
/// The trio is a UNIT. A model this fact cannot carry takes the effort and the
/// drift mark with it, because an effort beside a profile alias would read as
/// that profile's effort, and a drift mark with no model names no
/// disagreement. Nothing is escaped or truncated: a label ae cannot spell
/// EXACTLY is one it does not show.
fn observed_cells(identity: Option<&SeatIdentity>, rung: FactRung) -> (&str, &str, &'static str) {
    let empty = ("", "", "");
    if rung == FactRung::ClientOnly {
        return empty;
    }
    let Some(identity) = identity else {
        return empty;
    };
    let Some(model) = identity
        .model
        .as_deref()
        .filter(|model| model_is_writable(model))
    else {
        return empty;
    };
    if rung == FactRung::NoEffort {
        return (model, "", "");
    }
    (
        model,
        identity
            .effort
            .as_deref()
            .filter(|effort| crate::harness_state::is_effort_word(effort))
            .unwrap_or_default(),
        if identity.drift { "!" } else { "" },
    )
}

/// Whether a model label can be spelled in the fact EXACTLY as observed.
///
/// The separators and the tmux style bytes are the fact's own grammar, and a
/// vendor label is free text never promised to avoid them. A label carrying
/// one is dropped rather than escaped: an escape would be a second grammar for
/// the picker to get wrong, and the parser's arity check would refuse the
/// whole roster over one vendor's punctuation.
fn model_is_writable(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= tmux::PICKER_AGENTS_MAX_MODEL
        && !model.starts_with(' ')
        && !model.ends_with(' ')
        && model.bytes().all(|byte| {
            (b' '..=b'~').contains(&byte) && !matches!(byte, b'|' | b',' | b'#' | b':' | b';')
        })
}

/// The carried state for `key`, created on first sight.
pub(super) fn entry_mut<'a, V: Default>(list: &'a mut Vec<(String, V)>, key: &str) -> &'a mut V {
    let index = list
        .iter()
        .position(|(held, _)| held == key)
        .unwrap_or_else(|| {
            list.push((key.to_owned(), V::default()));
            list.len() - 1
        });
    let Some((_, value)) = list.get_mut(index) else {
        unreachable!("the index either came from this vector or was just pushed");
    };
    value
}

/// The descendancy for a pane, with BOTH unusable inputs mapped to `Unknown`.
pub(super) fn descendancy_of(
    table: Option<&[procs::Proc]>,
    pane_pid: Option<u32>,
    bin: Option<&str>,
) -> Descendancy {
    match (pane_pid, bin) {
        (Some(pid), Some(binary)) => procs::descendancy(table, pid, binary),
        _ => Descendancy::Unknown,
    }
}

/// The session's events in APPEND ORDER, oldest first.
pub(crate) fn read_events(meta_dir: &Path) -> Vec<Event> {
    let bytes = store::open(meta_dir).container();
    crate::event_text::read_lines(&bytes)
        .into_iter()
        .filter_map(|line| Event::parse_line(&String::from_utf8_lossy(line)).ok())
        .collect()
}

/// Whether a give-up whose `ref` names `destination` exactly is already in the
/// ledger — the idempotence that makes the give-up's two steps (append, then
/// rename) safe to interrupt: the ref is written uncapped and matched exactly,
/// never through the capped summary.
pub(super) fn announced(events: &[Event], destination: &str) -> bool {
    events.iter().any(|event| {
        event.action == crate::brief_retry::GAVE_UP_ACTION
            && event.reference.as_deref() == Some(destination)
    })
}

/// Whether the meta declares this session the fleet orchestrator.
pub(super) fn is_meta_agent(meta_bytes: &[u8]) -> bool {
    crate::meta::meta_agent_role(meta_bytes) == crate::meta::MetaAgentRole::Role
}

/// The session this meta directory serves — its `session=` key, or the
/// directory's own name, which is what the directory IS named.
pub(super) fn session_name(meta_bytes: &[u8], meta_dir: &Path) -> String {
    String::from_utf8_lossy(meta_bytes)
        .lines()
        .find_map(|line| line.strip_prefix("session="))
        .filter(|value| !value.is_empty())
        .map_or_else(
            || {
                meta_dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            },
            ToOwned::to_owned,
        )
}

/// The watchdog bar's glyph, from the cycle's COUNTS.
/// The watch segment of the bar.
///
/// A HEALTHY watch says only that it is watching: the counts are a monitoring
/// fact, and the bar is a hierarchy where the session, its windows and its
/// agents come first. The moment a pane is dead or stale the counts come back,
/// in the mark's own accent, because then they are the news.
pub(super) fn watch_segment(published: &Published<'_>) -> String {
    let look = published.look;
    let healthy = published.active == published.total;
    if healthy {
        return format!("#[fg={}]{}", look.palette.dim, published.bar);
    }
    format!(
        "#[fg={}]{} {}/{}#[default]",
        look.palette.accent(published.attention),
        published.bar,
        published.active,
        published.total,
    )
}

pub(super) fn bar_glyph(dead: usize, stale: usize, icons: bool) -> &'static str {
    if dead > 0 {
        Verdict::Dead.glyph(icons)
    } else if stale > 0 {
        Verdict::Stale.glyph(icons)
    } else {
        Verdict::Active.glyph(icons)
    }
}
