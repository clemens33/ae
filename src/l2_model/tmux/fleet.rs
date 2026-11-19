//! The fleet's tmux reads: the strip's panes and sessions, the watchdog's listing, and the picker's clients, agents, sessions, panes and pre-draw read.

use super::{
    FIELD_SEPARATOR, LOOK_FORMAT, LookOptions, Mark, SESSION_NAME_FORMAT, ServerId,
    interpret_display_value, interpret_look, interpret_sessions, is_view_session, server_args,
};

/// The format the fleet picker asks for: which session a pane belongs to, its
/// id, and the agent stamped on it.
pub const FLEET_PANE_FORMAT: &str = "#{session_name} | #{pane_id} | #{@ae_agent}";

/// How many [`FIELD_SEPARATOR`]-separated fields [`FLEET_PANE_FORMAT`] makes.
const FLEET_PANE_FIELDS: usize = 3;

/// One stamped pane of one session, as the caller's own server reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetPane {
    /// The session the pane belongs to.
    pub session: String,
    /// The `%<n>` pane id.
    pub pane: String,
    /// The `@ae_agent` stamp, empty on a pane no agent owns.
    pub agent: String,
}

/// The arguments listing EVERY pane on `server` — `-a`, deliberately, because
/// one listing answers "which of ae's sessions are reachable from this client,
/// and what are their panes" for the whole fleet at once.
#[must_use]
pub fn fleet_panes_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-a", "-F", FLEET_PANE_FORMAT].map(ToOwned::to_owned));
    args
}

/// What a completed fleet-pane listing means.
///
/// A line with the wrong field count is dropped rather than guessed at: the
/// last field may legitimately hold the separator, so the split is bounded and
/// a short line is corruption.
#[must_use]
pub fn interpret_fleet_panes(succeeded: bool, stdout: &str) -> Option<Vec<FleetPane>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            // Only the carriage return goes: an UNSTAMPED pane ends the line
            // with the separator and an empty field, and trimming that away
            // would drop the pane rather than report it stampless.
            .map(|line| line.trim_end_matches('\r'))
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let fields: Vec<&str> = line.splitn(FLEET_PANE_FIELDS, FIELD_SEPARATOR).collect();
                let [session, pane, agent] = fields.as_slice() else {
                    return None;
                };
                // A pane view's grouped session lists the real session's
                // panes a second time; the real session alone owns them.
                if is_view_session(session) {
                    return None;
                }
                Some(FleetPane {
                    session: (*session).to_owned(),
                    pane: (*pane).to_owned(),
                    agent: agent.trim_end().to_owned(),
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// The fleet strip's two reads.
// ---------------------------------------------------------------------------

/// The format the fleet strip asks `list-sessions` for: a session's name, the
/// `$<n>` id a click resolves through, and the attention rank its OWN watchdog
/// published.
///
/// The rank is what makes this an ae listing: a session with none is not one ae
/// watches, so the strip skips it rather than drawing a stranger.
pub const FLEET_SESSION_FORMAT: &str = "#{session_name} | #{session_id} | #{@ae_attn_rank}";

/// How many fields [`FLEET_SESSION_FORMAT`] yields.
const FLEET_SESSION_FIELDS: usize = 3;

/// The highest rank a session may publish — [`crate::theme::Mark`]'s own count.
const HIGHEST_RANK: u8 = 5;

/// One ae session as the fleet strip reads it off the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetSession {
    /// The session name.
    pub name: String,
    /// Its `$<n>` id.
    pub id: String,
    /// The published attention rank, proven to be one.
    pub rank: String,
}

/// The arguments listing every session on `server` with its published
/// attention.
#[must_use]
pub fn fleet_sessions_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-sessions", "-F", FLEET_SESSION_FORMAT].map(ToOwned::to_owned));
    args
}

/// What a completed fleet-session listing means.
///
/// A session with no rank is dropped: it is not an ae session, or its watchdog
/// has not run a cycle yet, and a strip that drew it would claim to know
/// something about it.
#[must_use]
pub fn interpret_fleet_sessions(succeeded: bool, stdout: &str) -> Option<Vec<FleetSession>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            .map(|line| line.trim_end_matches(['\r', '\n']))
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let fields: Vec<&str> =
                    line.splitn(FLEET_SESSION_FIELDS, FIELD_SEPARATOR).collect();
                let [name, id, rank] = fields.as_slice() else {
                    return None;
                };
                // PROVEN, not trusted. Every field of this row is rendered into
                // an option value the drawer reads `#[…]` out of, and the row
                // comes from a session ae did not necessarily create: a name
                // with a style directive in it would restyle the strip and tear
                // its click ranges. A session ae launched always passes.
                let (name, id, rank) = (name.trim(), id.trim(), rank.trim());
                let named = crate::session_launch::name::is_session_name(name);
                let identified = id.strip_prefix('$').is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                });
                let ranked = rank.parse::<u8>().is_ok_and(|rank| rank <= HIGHEST_RANK);
                (named && identified && ranked).then(|| FleetSession {
                    name: name.to_owned(),
                    id: id.to_owned(),
                    rank: rank.to_owned(),
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// The watchdog's own fleet listing: the strip's rows AND each session's look.
// ---------------------------------------------------------------------------

/// The listing the WATCHDOG asks `list-sessions` for: [`FLEET_SESSION_FORMAT`]'s
/// three fields, then the four [`LOOK_FORMAT`] carries.
///
/// ONE read, because a daemon that fills a watchdog-less peer's fleet strip must
/// draw it in THAT session's colours, and a second query per peer per tick is a
/// tmux process per peer per tick. The tail is `LOOK_FORMAT` VERBATIM, so the
/// look is parsed by the one look parser rather than by a second spelling of the
/// same four fields; `the_listing_format_ends_in_the_one_look_format` pins the
/// equality, and the literal is still written out because a format assembled by
/// `concat!` cannot be read at its use site.
pub const FLEET_LISTING_FORMAT: &str = "#{session_name} | #{session_id} | #{@ae_attn_rank} | #{@ae_icons} | #{@ae_palette} | #{@ae_look} | #{@ae_motion}";

/// How many fields [`FLEET_LISTING_FORMAT`] yields: three, then the look's four.
pub(super) const FLEET_LISTING_FIELDS: usize = FLEET_SESSION_FIELDS + 4;

/// One ae session as the watchdog's fleet listing reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetListingRow {
    /// The session name.
    pub name: String,
    /// Its `$<n>` id.
    pub id: String,
    /// The published attention rank, proven to be one — or `None` when the
    /// session published NO rank, which is a different fact from a bad one.
    /// Unlike [`interpret_fleet_sessions`], a rankless row survives the read:
    /// whether it is an ae session nobody is measuring or a stranger is settled
    /// by ae's own records, not by a tmux option a stranger could also set.
    pub rank: Option<String>,
    /// The look this session is drawn in, as its own options declare it.
    pub look: LookOptions,
}

/// The arguments listing every session on `server` with its attention AND its
/// look.
#[must_use]
pub fn fleet_listing_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(fleet_listing_words());
    args
}

/// The `list-sessions` command of [`fleet_listing_args`], without the server
/// words.
pub(super) fn fleet_listing_words() -> Vec<String> {
    ["list-sessions", "-F", FLEET_LISTING_FORMAT]
        .map(ToOwned::to_owned)
        .into()
}

/// What a completed [`fleet_listing_args`] run means.
#[must_use]
pub fn interpret_fleet_listing(succeeded: bool, stdout: &str) -> Option<Vec<FleetListingRow>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            .map(|line| line.trim_end_matches(['\r', '\n']))
            .filter(|line| !line.is_empty())
            .filter_map(fleet_listing_row)
            .collect(),
    )
}

/// One listing line, or `None` for a line ae will not read.
///
/// EXACT field count, never `splitn`: `@ae_palette` and the other look options
/// are human-settable and a session ae did not create may hold a
/// [`FIELD_SEPARATOR`] in its NAME, so a row carrying the separator inside a
/// value is DROPPED rather than read with every field behind it shifted one
/// place into its neighbour. Every field is then proven the same way the
/// three-field reader proves its own, because both are rendered into an option
/// value the drawer reads `#[…]` out of.
fn fleet_listing_row(line: &str) -> Option<FleetListingRow> {
    let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
    if fields.len() != FLEET_LISTING_FIELDS {
        return None;
    }
    let (name, id, rank) = (fields[0].trim(), fields[1].trim(), fields[2].trim());
    if !crate::session_launch::name::is_session_name(name) || !is_session_id(id) {
        return None;
    }
    // THREE outcomes, not two: no rank is a session that published none, a
    // rank ae recognises is a verdict, and anything else is a row ae will not
    // read at all — the same refusal the three-field reader makes by dropping
    // it, so neither reader can be talked into drawing a value it cannot name.
    let rank = match rank {
        "" => None,
        value if is_attention_rank(value) => Some(value.to_owned()),
        _ => return None,
    };
    // The ONE look parser, over the tail rejoined exactly as tmux wrote it.
    let look = interpret_look(true, &fields[FLEET_SESSION_FIELDS..].join(FIELD_SEPARATOR))?;
    Some(FleetListingRow {
        name: name.to_owned(),
        id: id.to_owned(),
        rank,
        look,
    })
}

/// Whether `id` is a tmux `$<n>` session id.
pub(super) fn is_session_id(id: &str) -> bool {
    id.strip_prefix('$')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `rank` is one of the ranks [`crate::theme::Mark`] publishes.
fn is_attention_rank(rank: &str) -> bool {
    rank.parse::<u8>().is_ok_and(|rank| rank <= HIGHEST_RANK)
}

// ---------------------------------------------------------------------------
// The fleet picker's live-server reads.
// ---------------------------------------------------------------------------

/// The explicit client, its current session id, its process and its drawable
/// dimensions.
///
/// The pid travels in the SAME row as the identity it belongs to: a second
/// listing could observe a replacement client and pair the old session id with
/// the new attachment, which is exactly what the resume pin must never do.
pub const PICKER_CLIENT_SESSION_FORMAT: &str =
    "#{client_name} | #{session_id} | #{client_pid} | #{client_height} | #{client_width}";

/// One exact picker client's live snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerClient {
    /// The captured rename-safe session id.
    pub session_id: String,
    /// The client process, as tmux reports it.
    pub pid: String,
    /// The client's terminal rows.
    pub height: usize,
    /// The client's terminal columns.
    pub width: usize,
}

/// List every attached client once so the picker can resolve its explicit
/// client to the session whose button owns the open marker.
#[must_use]
pub fn picker_client_sessions_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-clients", "-F", PICKER_CLIENT_SESSION_FORMAT].map(ToOwned::to_owned));
    args
}

/// The validated session id belonging to exactly one `client` row.
#[must_use]
pub fn interpret_picker_client_session(
    succeeded: bool,
    stdout: &str,
    client: &str,
) -> Option<PickerClient> {
    if !succeeded {
        return None;
    }
    let mut matches = stdout.lines().filter_map(|line| {
        let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
        let [found, session_id, pid, height, width] = fields.as_slice() else {
            return None;
        };
        if *found != client || !session_id_is_valid(session_id) || !is_decimal(pid) {
            return None;
        }
        Some(PickerClient {
            session_id: (*session_id).to_owned(),
            pid: (*pid).to_owned(),
            height: height.parse().ok()?,
            width: width.parse().ok()?,
        })
    });
    let snapshot = matches.next()?;
    matches.next().is_none().then_some(snapshot)
}

/// The explicit client's name, its process, the session it views and its
/// drawable dimensions.
///
/// A client NAME is a tty path, and a tty path is reused: a client that
/// detached and another that attached on the same terminal answer to the same
/// name. The pid separates them, so a confirmation built for one attachment
/// cannot be applied by its successor.
pub const MENU_CLIENT_FORMAT: &str = "#{client_name} | #{client_pid} | #{session_id} | #{session_name} | #{client_height} | #{client_width}";

/// How many [`FIELD_SEPARATOR`]-separated fields [`MENU_CLIENT_FORMAT`] makes.
const MENU_CLIENT_FIELDS: usize = 6;

/// One exact menu client's live snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuClient {
    /// The client process, as tmux reports it.
    pub pid: String,
    /// The session this client is currently viewing.
    pub session_id: String,
    /// The validated durable session name selecting this session's overlay.
    pub session_name: String,
    /// The client's terminal rows.
    pub height: usize,
    /// The client's terminal columns.
    pub width: usize,
}

/// List every attached client once so a menu can resolve its explicit client.
#[must_use]
pub fn menu_clients_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-clients", "-F", MENU_CLIENT_FORMAT].map(ToOwned::to_owned));
    args
}

/// The one row belonging to exactly one `client`, or `None`.
///
/// Two rows for one name is an ambiguity, not a choice: this returns nothing
/// rather than picking one of them.
#[must_use]
pub fn interpret_menu_client(succeeded: bool, stdout: &str, client: &str) -> Option<MenuClient> {
    if !succeeded {
        return None;
    }
    let mut matches = stdout.lines().filter_map(|line| {
        let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
        if fields.len() != MENU_CLIENT_FIELDS {
            return None;
        }
        let [found, pid, session_id, session_name, height, width] = fields.as_slice() else {
            return None;
        };
        if *found != client
            || !session_id_is_valid(session_id)
            || !crate::session_launch::name::is_session_name(session_name)
            || !is_decimal(pid)
        {
            return None;
        }
        Some(MenuClient {
            pid: (*pid).to_owned(),
            session_id: (*session_id).to_owned(),
            session_name: (*session_name).to_owned(),
            height: height.parse().ok()?,
            width: width.parse().ok()?,
        })
    });
    let snapshot = matches.next()?;
    matches.next().is_none().then_some(snapshot)
}

/// The running tmux server's process and the moment it started.
///
/// tmux 3.4 records `start_time` once, after it daemonises, and renders it as
/// whole epoch seconds. This pair is a PRACTICAL restart check, not a lifetime
/// token: neither field identifies a server on its own, and the pair itself
/// repeats when a replacement server takes a reused pid within the same second.
/// That residual is the accepted one; the alternative is a new persisted
/// protocol, which is not worth its own hostile parser.
pub const SERVER_IDENTITY_FORMAT: &str = "#{pid} | #{start_time}";

/// One server's identity pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerIdentity {
    /// The server process id.
    pub pid: String,
    /// The epoch second the server started.
    pub start: String,
}

/// Ask `server` for its identity pair.
#[must_use]
pub fn server_identity_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", SERVER_IDENTITY_FORMAT].map(ToOwned::to_owned));
    args
}

/// The identity pair a completed [`server_identity_args`] run printed.
#[must_use]
pub fn interpret_server_identity(succeeded: bool, stdout: &str) -> Option<ServerIdentity> {
    if !succeeded {
        return None;
    }
    let line = stdout.lines().next()?;
    let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
    let [pid, start] = fields.as_slice() else {
        return None;
    };
    (is_decimal(pid) && is_decimal(start)).then(|| ServerIdentity {
        pid: (*pid).to_owned(),
        start: (*start).to_owned(),
    })
}

/// A nonempty run of ASCII digits, which is every numeric field tmux prints
/// here and the only shape this module admits as one.
pub(crate) fn is_decimal(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// One picker row, identity first and its free-text goal last.
///
/// The first three fields admit the row. The remaining fields are display or
/// navigation hints and may be absent. Keeping the goal last lets
/// [`interpret_picker_sessions`] use `splitn`, so a literal pipe in operator
/// text cannot shift a pane id into another field.
pub const PICKER_SESSION_FORMAT: &str = "#{session_name} | #{session_id} | #{@ae_attn_rank} | #{@ae_attn_glyph} | #{@ae_main_pane} | #{s/#{l:[|[:cntrl:]]}//:@ae_branch_name} | #{s/#{l:[|[:cntrl:]]}/!/:@ae_agents} | #{session_activity} | #{session_created} | #{@ae_goal_status}";

/// How many fields [`PICKER_SESSION_FORMAT`] yields.
const PICKER_SESSION_FIELDS: usize = 10;

/// How many fields the format yielded before the two session times were added
/// ahead of the goal: such a line keeps its goal where it was.
const PICKER_LEGACY_FIELDS: usize = 8;

/// Maximum bytes accepted from the watchdog-owned `@ae_agents` fact.
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
    /// Whether the watchdog proved a live job of the seat's tool: only a `v3`
    /// fact can say so, and only as the RAW fact — the mark decides whether it
    /// shows ([`Mark::with_task`]).
    pub task: bool,
}

impl PickerAgent {
    /// The shared static picker mark for this verdict word, with the seat's
    /// proven job composed in by the one rule the watchdog uses too.
    #[must_use]
    pub fn mark(&self) -> Mark {
        picker_agent_mark(&self.state)
            .unwrap_or(Mark::Stale)
            .with_task(self.task)
    }
}

/// Which `@ae_agents` grammar one value announces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentsGrammar {
    /// `name:profile:state:pane` — no observed cells.
    V1,
    /// v1's four fields plus `client:model:effort:drift`.
    V2,
    /// v2's eight fields plus `task`: `1` for a proven job, else empty.
    V3,
}

impl AgentsGrammar {
    /// How many colon-separated fields one entry must have. EXACTLY: a v1
    /// entry with a fifth field is damage, not a forward-compatible row.
    const fn entry_fields(self) -> usize {
        match self {
            Self::V1 => 4,
            Self::V2 => 8,
            Self::V3 => 9,
        }
    }
}

/// Parse one bounded, versioned `@ae_agents` value.
///
/// EVERY grammar is accepted, because versions run side by side: `ae upgrade`
/// restarts the watchdogs it can reach, a skipped session keeps its old core,
/// and a checkout core can share one server. A v1 fact read here simply proves
/// no observed cells, and a v1 or v2 fact no job. The converse needs no shim: a
/// newer fact read by an OLD core fails its exact version check, yielding
/// `None` and today's `agents: unavailable` row.
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
        Some("v3") => AgentsGrammar::V3,
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
            AgentsGrammar::V2 | AgentsGrammar::V3 => [fields[4], fields[5], fields[6], fields[7]],
        };
        let task = match grammar {
            AgentsGrammar::V1 | AgentsGrammar::V2 => "",
            AgentsGrammar::V3 => fields[8],
        };
        if !crate::config::is_agent_name(name)
            || !crate::config::is_config_key(profile)
            || picker_agent_mark(state).is_none()
            || (!pane.is_empty() && !pane_id_is_valid(pane))
            || !observed_cells_are_coherent(client, model, effort, drift)
            || !matches!(task, "" | "1")
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
            task: task == "1",
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
    if !client.is_empty()
        && !crate::tool::is_client_token(client)
        && !crate::config::is_client_label(client)
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
        && (effort.is_empty() || crate::harness_state::is_effort_word(effort))
        && (drift.is_empty() || drift == DRIFT_MARK)
}

pub(super) fn picker_agent_mark(state: &str) -> Option<Mark> {
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

/// One live ae session admitted into the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerSession {
    /// The allowlisted session name.
    pub name: String,
    /// The tmux `$<n>` id captured with the row, used as its rename-safe target.
    pub id: String,
    /// The proven attention rank.
    pub rank: u8,
    /// The already-rendered attention glyph; empty when an older session lacks it.
    pub glyph: String,
    /// The recorded lead pane hint; empty when an older session lacks it.
    pub main_pane: String,
    /// The reader-sanitized branch fact; empty when unset or unavailable.
    pub branch: String,
    /// The raw watchdog-owned roster snapshot; parsed at draw time for age.
    pub agents: String,
    /// tmux's `session_activity`: the last time a client attached, switched
    /// in, or gave input here. Pane output and ae's own commands never move it.
    pub activity: Option<i64>,
    /// tmux's `session_created`, which `session_activity` starts at.
    pub created_at: Option<i64>,
    /// The already-bounded status goal; empty when unset or unavailable.
    pub goal: String,
}

impl PickerSession {
    /// The last moment a human touched the session, or `None` for one nobody
    /// has: tmux starts `session_activity` at the creation second, so only an
    /// activity past it is a client's.
    #[must_use]
    pub fn touched(&self) -> Option<i64> {
        let (activity, created) = (self.activity?, self.created_at?);
        (activity > created).then_some(activity)
    }

    /// The session's creation order, encoded in tmux's monotonically assigned id.
    #[must_use]
    pub fn created(&self) -> u64 {
        self.id
            .strip_prefix('$')
            .and_then(|digits| digits.parse().ok())
            .unwrap_or(u64::MAX)
    }
}

/// The arguments reading every picker row from the live server in one call.
#[must_use]
pub fn picker_sessions_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-sessions", "-F", PICKER_SESSION_FORMAT].map(ToOwned::to_owned));
    args
}

/// Parse the live picker rows.
///
/// Name, session id and rank are identity: a malformed one rejects the row,
/// matching the fleet-strip reader. Missing display fields become empty. A
/// failed listing remains `None`, distinct from a successful empty server.
#[must_use]
pub fn interpret_picker_sessions(succeeded: bool, stdout: &str) -> Option<Vec<PickerSession>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let times = picker_times(line);
                let mut fields = line.splitn(
                    if times.is_some() {
                        PICKER_SESSION_FIELDS
                    } else {
                        PICKER_LEGACY_FIELDS
                    },
                    FIELD_SEPARATOR,
                );
                let name = fields.next().unwrap_or_default().trim();
                let id = fields.next().unwrap_or_default().trim();
                let rank = fields.next().unwrap_or_default().trim();
                let named = crate::session_launch::name::is_session_name(name);
                let identified = session_id_is_valid(id);
                let rank = rank.parse::<u8>().ok().filter(|rank| *rank <= HIGHEST_RANK);
                (named && identified).then_some(())?;
                let rank = rank?;
                let glyph = fields.next().unwrap_or_default().trim().to_owned();
                let main_pane = fields.next().unwrap_or_default().trim().to_owned();
                let branch = fields.next().unwrap_or_default().trim().to_owned();
                let agents = fields.next().unwrap_or_default().trim().to_owned();
                if times.is_some() {
                    fields.nth(1);
                }
                let (activity, created_at) = times.unwrap_or_default();
                Some(PickerSession {
                    name: name.to_owned(),
                    id: id.to_owned(),
                    rank,
                    glyph,
                    main_pane,
                    branch,
                    agents,
                    activity,
                    created_at,
                    goal: fields.next().unwrap_or_default().trim().to_owned(),
                })
            })
            .collect(),
    )
}

/// The `(session_activity, session_created)` of a line in the CURRENT shape:
/// ten pieces whose 8th and 9th are each empty or digits. Anything else is a
/// line from before the two times joined the format, whose 8th piece is the
/// goal, pipes and all — a goal that merely looks like two numbers and a
/// tail is the one line this reads wrong, and only a test can write one.
fn picker_times(line: &str) -> Option<(Option<i64>, Option<i64>)> {
    let pieces: Vec<&str> = line
        .splitn(PICKER_SESSION_FIELDS, FIELD_SEPARATOR)
        .collect();
    if pieces.len() != PICKER_SESSION_FIELDS {
        return None;
    }
    let time = |piece: &str| {
        let piece = piece.trim();
        if piece.is_empty() {
            Some(None)
        } else if is_decimal(piece) {
            Some(piece.parse::<i64>().ok().filter(|moment| *moment > 0))
        } else {
            None
        }
    };
    Some((time(pieces[7])?, time(pieces[8])?))
}

/// The one build-time membership snapshot: session id then pane id.
pub const PICKER_PANE_FORMAT: &str = "#{session_id} | #{pane_id}";

/// One pane and the session that owns it in the picker snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerPane {
    /// The tmux `$<n>` session id.
    pub session_id: String,
    /// The tmux `%<n>` pane id.
    pub pane: String,
}

/// The arguments reading every pane membership from the live server once.
#[must_use]
pub fn picker_panes_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-a", "-F", PICKER_PANE_FORMAT].map(ToOwned::to_owned));
    args
}

/// Parse the membership snapshot, dropping only malformed identities.
#[must_use]
pub fn interpret_picker_panes(succeeded: bool, stdout: &str) -> Option<Vec<PickerPane>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let mut fields = line.splitn(2, FIELD_SEPARATOR);
                let session_id = fields.next().unwrap_or_default().trim();
                let pane = fields.next().unwrap_or_default().trim();
                (session_id_is_valid(session_id) && pane_id_is_valid(pane)).then(|| PickerPane {
                    session_id: session_id.to_owned(),
                    pane: pane.to_owned(),
                })
            })
            .collect(),
    )
}

pub(crate) fn session_id_is_valid(id: &str) -> bool {
    id.strip_prefix('$')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// A `#{pane_pid}` cell: decimal digits with no sign, no leading zero and not
/// 0, fitting a `u32`. Anything else is no pid.
pub(super) fn pane_pid(cell: &str) -> Option<u32> {
    let digits = cell.bytes().all(|b| b.is_ascii_digit()) && !cell.starts_with('0');
    digits.then(|| cell.parse().ok()).flatten()
}

pub(crate) fn pane_id_is_valid(pane: &str) -> bool {
    pane.strip_prefix('%')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

// ---------------------------------------------------------------------------
// The picker's ONE pre-draw read.
// ---------------------------------------------------------------------------

/// The row prefix and the completion marker of every command in
/// [`picker_read_args`].
///
/// tmux runs a `;`-separated command list in ONE server connection, so the
/// picker's whole read costs one process. Rows would blur together without a
/// per-command prefix, and a line carrying neither a prefix nor a marker is
/// refused rather than read as somebody else's rows; `ae-picker!<kind>` after a
/// command's rows PROVES that command completed, because tmux stops the list at
/// the first failure — so a section without its marker is unknown, never empty.
/// All printable ASCII, so no format rule is bent.
pub(super) const READ_STEM: &str = "ae-picker:";
pub(super) const READ_DONE_STEM: &str = "ae-picker!";
pub(super) const READ_CLIENTS: char = 'c';
pub(super) const READ_SESSIONS: char = 's';
const READ_PANES: char = 'p';
const READ_NAMES: char = 'n';
const READ_IDENTITY: char = 'i';
const READ_SOCKET: char = 'k';
pub(super) const READ_LOOK: char = 'l';

/// Which field of [`PickerRead`] a kind letter fills.
pub(super) fn read_section(kind: char) -> Option<usize> {
    [
        READ_CLIENTS,
        READ_SESSIONS,
        READ_PANES,
        READ_NAMES,
        READ_IDENTITY,
        READ_SOCKET,
        READ_LOOK,
    ]
    .iter()
    .position(|known| *known == kind)
}

/// Everything the picker's ONE read carried; a field is absent when its command
/// did not complete.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerRead {
    /// The invoking client resolved to exactly one live session.
    pub client: Option<PickerClient>,
    /// Every live session, pane membership and bare session name, as their
    /// parsers admit them.
    pub sessions: Option<Vec<PickerSession>>,
    pub panes: Option<Vec<PickerPane>>,
    pub names: Option<Vec<String>>,
    /// The server's own identity pair and socket path.
    pub identity: Option<ServerIdentity>,
    pub socket: Option<String>,
    /// The invoking client's session look.
    pub look: Option<LookOptions>,
}

/// The one tmux invocation carrying every picker READ before its draw.
///
/// One connection: the client snapshot, the session rows, the pane membership,
/// the bare name list, the server identity, its socket path and the invoking
/// client's look, each behind its own line prefix and followed by its completion
/// marker. tmux's own `;` separates the commands — this list never passes
/// through a shell, and the `@ae_menu_open` marker write is deliberately NOT
/// here: a write does not ride a read.
#[must_use]
pub fn picker_read_args(server: &ServerId, client: &str) -> Vec<String> {
    let reads: [(char, &[&str], String); 7] = [
        (
            READ_CLIENTS,
            &["list-clients", "-F"],
            format!("{READ_STEM}{READ_CLIENTS}|{PICKER_CLIENT_SESSION_FORMAT}"),
        ),
        (
            READ_SESSIONS,
            &["list-sessions", "-F"],
            format!("{READ_STEM}{READ_SESSIONS}|{PICKER_SESSION_FORMAT}"),
        ),
        (
            READ_PANES,
            &["list-panes", "-a", "-F"],
            format!("{READ_STEM}{READ_PANES}|{PICKER_PANE_FORMAT}"),
        ),
        (
            READ_NAMES,
            &["list-sessions", "-F"],
            format!("{READ_STEM}{READ_NAMES}|{SESSION_NAME_FORMAT}"),
        ),
        (
            READ_IDENTITY,
            &["display-message", "-p"],
            format!("{READ_STEM}{READ_IDENTITY}|{SERVER_IDENTITY_FORMAT}"),
        ),
        (
            READ_SOCKET,
            &["display-message", "-p"],
            format!("{READ_STEM}{READ_SOCKET}|#{{socket_path}}"),
        ),
        // The client-targeted read goes LAST: it is the only command that can
        // fail while the server is alive, and tmux stops the list there, so
        // nothing that must run waits behind it.
        (
            READ_LOOK,
            &["display-message", "-p", "-c", client],
            format!("{READ_STEM}{READ_LOOK}|{LOOK_FORMAT}"),
        ),
    ];
    let mut queue: Vec<String> = Vec::new();
    for (kind, words, format) in reads {
        let mut command: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
        command.push(format);
        let done = vec![
            "display-message".to_owned(),
            "-p".to_owned(),
            format!("{READ_DONE_STEM}{kind}"),
        ];
        for words in [command, done] {
            if !queue.is_empty() {
                queue.push(";".to_owned());
            }
            queue.extend(words);
        }
    }
    let mut args = server_args(server);
    args.extend(queue);
    args
}

/// Split and interpret the combined output of [`picker_read_args`].
///
/// `None` when any non-empty line carries NEITHER a row prefix nor a completion
/// marker: the output then belongs to something other than this read, and a
/// partial roster is worse than none. An empty line is skipped — it can carry
/// no row. A section whose marker never arrived is left absent, never empty.
#[must_use]
pub fn interpret_picker_read(stdout: &str, client: &str) -> Option<PickerRead> {
    let mut rows: [String; 7] = Default::default();
    let mut complete = [false; 7];
    for line in stdout.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some(marker) = line.strip_prefix(READ_DONE_STEM) {
            if marker.len() != 1 {
                return None;
            }
            let kind = marker.chars().next()?;
            complete[read_section(kind)?] = true;
            continue;
        }
        let rest = line.strip_prefix(READ_STEM)?;
        let mut characters = rest.chars();
        let kind = characters.next()?;
        let body = characters.as_str().strip_prefix('|')?;
        let section = read_section(kind)?;
        rows[section].push_str(body);
        rows[section].push('\n');
    }
    let section = |index: usize| complete[index].then(|| rows[index].as_str());
    Some(PickerRead {
        client: section(0).and_then(|rows| interpret_picker_client_session(true, rows, client)),
        sessions: section(1).and_then(|rows| interpret_picker_sessions(true, rows)),
        panes: section(2).and_then(|rows| interpret_picker_panes(true, rows)),
        names: section(3).and_then(|rows| interpret_sessions(true, rows).ok()),
        identity: section(4).and_then(|rows| interpret_server_identity(true, rows)),
        socket: section(5).and_then(|rows| interpret_display_value(true, rows)),
        look: section(6).and_then(|rows| interpret_look(true, rows)),
    })
}
