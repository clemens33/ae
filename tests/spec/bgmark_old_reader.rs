// Frozen pre-v3 reader from 6b9a7642 src/tmux.rs. Only module imports adapt.
#[allow(dead_code, reason = "frozen pre-v3 compatibility reference")]
mod bgmark_old_reader {
    use crate::theme::Mark;
    use crate::{config, harness_state, tool};
    fn pane_id_is_valid(pane: &str) -> bool {
        pane.strip_prefix("%")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    }
    pub const PICKER_AGENTS_MAX_BYTES: usize = 4_096;

    /// Maximum roster entries accepted from one `@ae_agents` fact.
    pub const PICKER_AGENTS_MAX_COUNT: usize = 64;

    /// Highest watchdog cadence an `@ae_agents` fact may carry.
    pub const PICKER_AGENTS_MAX_INTERVAL_SECS: u64 = 3_600;

    /// Maximum bytes accepted from one entry's observed model label.
    ///
    /// The label is free text scraped from a pane — the only field here whose
    /// vocabulary is the vendor's rather than ae's — so it is the one that needs
    /// a length rule at all. 32 holds every model the measured fleet draws
    /// (`DeepSeek V4.1 Flash OpenRouter` is the longest at 30) with room for a
    /// rename, and keeps 64 of them from crowding a roster out of
    /// [`PICKER_AGENTS_MAX_BYTES`] on their own.
    pub const PICKER_AGENTS_MAX_MODEL: usize = 32;

    /// One strictly parsed agent row from the watchdog-owned session fact.
    ///
    /// The first four fields are the v1 contract and admit the row. The observed
    /// cells below them arrive only from a `v2` fact, are DISPLAY facts, and are
    /// coherent by construction: a model implies a client, and an effort or a
    /// drift mark implies a model ([`observed_cells_are_coherent`]).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PickerAgent {
        /// The allowlisted agent identity.
        pub name: String,
        /// The allowlisted recorded profile alias.
        pub profile: String,
        /// The watchdog verdict word.
        pub state: String,
        /// The captured pane id, empty when that roster seat has no pane.
        pub pane: String,
        /// The seat's client cell — the operator's `[clients]` label fully
        /// written, or a legacy adapter token — empty on a v1 fact.
        pub client: String,
        /// The model the seat's own frame drew, empty when nothing was observed.
        pub model: String,
        /// The effort that frame drew, empty when the frame proved none.
        pub effort: String,
        /// Whether the observed model differs from the profile's own pin.
        pub drift: bool,
    }

    /// Which `@ae_agents` grammar one value announces.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum AgentsGrammar {
        /// `name:profile:state:pane` — no observed cells.
        V1,
        /// v1's four fields plus `client:model:effort:drift`.
        V2,
    }

    impl AgentsGrammar {
        /// How many colon-separated fields one entry must have. EXACTLY: a v1
        /// entry with a fifth field is damage, not a forward-compatible row.
        const fn entry_fields(self) -> usize {
            match self {
                Self::V1 => 4,
                Self::V2 => 8,
            }
        }
    }

    /// Parse one bounded, versioned `@ae_agents` value.
    ///
    /// BOTH grammars are accepted, because versions run side by side: `ae upgrade`
    /// restarts the watchdogs it can reach, a skipped session keeps its old core,
    /// and a checkout core can share one server. A v1 fact read here simply proves
    /// no observed cells. The converse needs no shim: a v2 fact read by an OLD
    /// core fails its exact version check, yielding `None` and today's
    /// `agents: unavailable` row.
    ///
    /// Any malformed byte rejects the whole snapshot. `None` therefore means
    /// unavailable, never a partially trusted roster. A snapshot becomes stale
    /// only once it is older than two of its own watchdog intervals, or more than
    /// one interval ahead of the reader's clock.
    #[must_use]
    pub fn parse_picker_agents(raw: &str, now_epoch: i64) -> Option<Vec<PickerAgent>> {
        if raw.is_empty()
            || raw.len() > PICKER_AGENTS_MAX_BYTES
            || !raw.bytes().all(|byte| (b' '..=b'~').contains(&byte))
            || raw.bytes().any(|byte| matches!(byte, b'|' | b',' | b'#'))
        {
            return None;
        }
        let mut parts = raw.split(';');
        let grammar = match parts.next() {
            Some("v1") => AgentsGrammar::V1,
            Some("v2") => AgentsGrammar::V2,
            _ => return None,
        };
        let epoch = parts
            .next()?
            .parse::<i64>()
            .ok()
            .filter(|epoch| *epoch >= 0)?;
        let interval_secs = parts
            .next()?
            .parse::<u64>()
            .ok()
            .filter(|interval| (1..=PICKER_AGENTS_MAX_INTERVAL_SECS).contains(interval))?;
        let interval = i64::try_from(interval_secs).ok()?;
        let max_age = interval.saturating_mul(2);
        if now_epoch.saturating_sub(epoch) > max_age || epoch.saturating_sub(now_epoch) > interval {
            return None;
        }
        let mut agents = Vec::new();
        for entry in parts {
            if entry.is_empty() || agents.len() == PICKER_AGENTS_MAX_COUNT {
                return None;
            }
            let fields: Vec<&str> = entry.split(':').collect();
            if fields.len() != grammar.entry_fields() {
                return None;
            }
            // Past the arity check the four v1 fields are always there, and the
            // four observed cells are empty on a v1 row.
            let [name, profile, state, pane] = [fields[0], fields[1], fields[2], fields[3]];
            let [client, model, effort, drift] = match grammar {
                AgentsGrammar::V1 => ["", "", "", ""],
                AgentsGrammar::V2 => [fields[4], fields[5], fields[6], fields[7]],
            };
            if !config::is_agent_name(name)
                || !config::is_config_key(profile)
                || picker_agent_mark(state).is_none()
                || (!pane.is_empty() && !pane_id_is_valid(pane))
                || !observed_cells_are_coherent(client, model, effort, drift)
                || agents.iter().any(|agent: &PickerAgent| agent.name == *name)
            {
                return None;
            }
            agents.push(PickerAgent {
                name: name.to_owned(),
                profile: profile.to_owned(),
                state: state.to_owned(),
                pane: pane.to_owned(),
                client: client.to_owned(),
                model: model.to_owned(),
                effort: effort.to_owned(),
                drift: drift == DRIFT_MARK,
            });
        }
        (!agents.is_empty()).then_some(agents)
    }

    /// The drift field's only nonempty spelling, and the byte the picker draws.
    ///
    /// The picker reads this fact through a tmux format that rewrites `|` and
    /// control bytes to `!`, so a `!` here is indistinguishable from a
    /// hand-written `|`. That costs nothing: the writer emits neither, and anyone
    /// who can edit the option could write the mark directly anyway.
    const DRIFT_MARK: &str = "!";

    /// Whether one entry's four observed cells can have come from a writer.
    ///
    /// They are a UNIT: the writer publishes a model only with the client that
    /// drew it, and empties the trio together when it cannot represent one, so a
    /// model without a client — or an effort or a drift mark without a model —
    /// is damage. Rejecting it here spares every reader a rendering for a state
    /// ae does not produce.
    fn observed_cells_are_coherent(client: &str, model: &str, effort: &str, drift: &str) -> bool {
        // The full `[clients]` label, or a legacy adapter token — same `v2`
        // grammar, so an old core fails into `agents: unavailable`, never a
        // misrender, and a new core still reads an old watchdog's rows.
        if !client.is_empty() && !tool::is_client_token(client) && !config::is_client_label(client)
        {
            return false;
        }
        if model.is_empty() {
            return effort.is_empty() && drift.is_empty();
        }
        !client.is_empty()
            && model.len() <= PICKER_AGENTS_MAX_MODEL
            && !model.starts_with(' ')
            && !model.ends_with(' ')
            && (effort.is_empty() || harness_state::is_effort_word(effort))
            && (drift.is_empty() || drift == DRIFT_MARK)
    }

    fn picker_agent_mark(state: &str) -> Option<Mark> {
        match state {
            "dead" => Some(Mark::Dead),
            "waiting-user" | "blocked" | "throttled" | "limit" | "wedged" | "prompt" => {
                Some(Mark::NeedsYou)
            }
            // A FRESH `waiting-agent` is quiet with its own seventh mark. An
            // ESCALATED one reaches this map as `blocked` above.
            "waiting-agent" => Some(Mark::WaitingAgent),
            "working" | "sweeping" | "busy" => Some(Mark::Working),
            "done" => Some(Mark::Done),
            "stale" | "starting" | "unreachable" => Some(Mark::Stale),
            "idle" => Some(Mark::Idle),
            _ => None,
        }
    }
}
#[test]
fn bgmark_old_core_reader_rejects_v3_without_reinterpreting_fields() {
    assert!(
        bgmark_old_reader::parse_picker_agents(
            "v2;2000;60;lead:p:waiting-agent:%3:claude:::",
            2000
        )
        .is_some()
    );
    assert!(
        bgmark_old_reader::parse_picker_agents(
            "v3;2000;60;lead:p:waiting-agent:%3:claude::::1",
            2000
        )
        .is_none()
    );
}
