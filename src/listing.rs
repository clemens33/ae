//! The two renderings of `ae list` — SC-017f, SC-017g, SC-017h, SC-509.
//!
//! # One selection, two renderings
//!
//! **SC-017f** — `--json` honours the active filters. That is why nothing here
//! filters: [`crate::filters::Selection::select`] runs ONCE and both renderings
//! consume its result. A second selection path is a second chance to disagree,
//! and the row exists precisely to forbid the disagreement.
//!
//! The machine rendering is [`crate::digest::Digest`] verbatim — SC-509's
//! versioned object, SC-506's always-closing document. This module calls it; it
//! does not re-derive the shape.
//!
//! # The injected world
//!
//! [`World`] is the phase boundary. Enumeration is SC-017j's (phase 1),
//! liveness SC-017k/l's (phase 2), and both are ratified and implemented — so
//! the sessions arriving as a parameter is no longer a placeholder for an
//! undecided surface but the thing that keeps THIS module unable to re-derive
//! them. Presentation gets facts and presents them.
//!
//! The shipped binary wires that source itself: [`crate::run`] derives the state
//! root, discovers, classifies and renders. The old "no session source is
//! wired" refusal is gone.
//!
//! # PROVISIONAL — the tabular LAYOUT is not ratified
//!
//! **SC-017h names content, not form.** Its authority (commands.md:56-59) is a
//! single sentence: a tabular view "with per-agent health, declared state, and
//! a session-level `attn:<reason>` marker when a session needs attention". No
//! columns, no field order, no widths, no empty-listing text. Ratified with the
//! seats for this slice:
//!
//! * what [`table`] SHOWS is pinned — the row's three nouns, and nothing else;
//! * what it LOOKS LIKE is **provisional and unratified** — no test asserts its
//!   exact bytes, because layout bytes are a seat decision informed by parity
//!   evidence rather than an implementer's taste.
//!
//! Two things follow, and both are deliberate. An agent's own attention reason
//! is NOT rendered: SC-017h names three nouns and `attn:` is the SESSION-level
//! marker (SC-017g's rollup). And an empty selection prints nothing at all —
//! the least-invented answer, since no row gives the text for "no sessions".

use crate::digest::{Digest, SessionEntry};
use crate::filters::ListArgs;
use crate::inventory::FailedSource;
use crate::liveness::Snapshot;
use crate::session::SessionRuntime;
use crate::time::Timestamp;

/// The facts a listing needs that no session directory holds.
///
/// The completed phase-2 snapshot, projected for presentation: sessions with
/// their classified status, and SC-017o's loss facts. See the module docs for
/// why it arrives as a value rather than being fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct World {
    /// The moment the listing is taken as of — SC-509's `generated_at`, and
    /// SC-017e's "now" for the activity window.
    pub now: Timestamp,
    /// Every session known to the caller, before any filter runs.
    pub sessions: Vec<SessionEntry>,
    /// **SC-017o** — HOW MANY distinct logical sources failed to enumerate.
    ///
    /// A property of the SNAPSHOT, not of the selection: filtering changes which
    /// sessions are shown and can never change whether ae managed to look
    /// everywhere. It passes through [`render`] untouched, so every filter's
    /// document carries the same answer and every human view the same warning.
    ///
    /// **A COUNT, AND NOT THE PATHS, ON PURPOSE.** SC-017o's human surface needs
    /// the number and its machine surface needs the boolean; neither needs a
    /// path. And what presentation cannot NAME, it cannot re-derive: with no
    /// state root and no record path anywhere in its input, a renderer that
    /// wanted to re-check a fact has nothing to open. That is a narrower claim
    /// than "no syscall can occur" — a gratuitous one stays expressible — but it
    /// is the claim that matters, because a re-derivation of the facts being
    /// presented needs an address for them.
    ///
    /// [`World::inventory_complete`] derives the boolean from this, so the two
    /// can never disagree.
    pub losses: usize,
}

impl World {
    /// A world of `sessions` as of `now`, from a COMPLETE enumeration.
    #[must_use]
    pub fn new(now: Timestamp, sessions: Vec<SessionEntry>) -> Self {
        Self {
            now,
            sessions,
            losses: 0,
        }
    }

    /// The same world, carrying what SC-017o's loss facts amount to.
    ///
    /// Takes the FACTS and keeps their cardinality, through the same counter the
    /// boundary uses — so "one source recorded twice is one loss" is decided in
    /// one place rather than twice.
    #[must_use]
    pub fn with_losses(mut self, losses: &[FailedSource]) -> Self {
        self.losses = distinct_losses(losses);
        self
    }

    /// SC-017o's `inventory_complete` — derived, never stored.
    #[must_use]
    pub const fn inventory_complete(&self) -> bool {
        self.losses == 0
    }

    /// How many DISTINCT logical sources failed — SC-017o's human count.
    #[must_use]
    pub const fn loss_count(&self) -> usize {
        self.losses
    }
}

/// The phase-3 boundary — **the first operation on a classified snapshot**.
///
/// This type exists so the boundary is a PRODUCTION fact rather than a line in a
/// test. An earlier version had the suite append a "presentation enter" marker
/// to its own log AFTER calling the projection, which cannot establish a
/// sequence: everything the projection did happened before the marker and was
/// invisible to it. A test log is narration; an invoked entry point is an
/// observation.
///
/// [`Presentation::enter`] does no filtering, no sorting and no formatting — it
/// only borrows the snapshot — so no phase-3 work can precede it.
/// [`Presentation::at_entry`] is what the snapshot said AT that moment, in the
/// order it said it, which is what makes reordering or dropping anything at the
/// boundary observable.
///
/// **There is no wrapper around this, deliberately.** A `world_of(snapshot, …)`
/// convenience used to exist, and it was a place work could hide: a caller that
/// mangled a clone and then entered with the changed value was invisible to a
/// test that entered directly, because the test was below the real route. The
/// route and the boundary are now the same call, so "below it" is not a place.
pub struct Presentation<'a> {
    snapshot: &'a Snapshot,
}

impl<'a> Presentation<'a> {
    /// Enter presentation with `snapshot`. Nothing else happens here.
    #[must_use]
    pub const fn enter(snapshot: &'a Snapshot) -> Self {
        Self { snapshot }
    }

    /// The identities and statuses this boundary received, IN ORDER.
    ///
    /// Unsorted on purpose: a fingerprint that sorted its own input could not
    /// tell an untouched snapshot from one reordered before the boundary.
    #[must_use]
    pub fn at_entry(&self) -> Vec<(String, &'static str)> {
        self.snapshot
            .sessions
            .iter()
            .map(|classified| {
                (
                    classified.candidate.name.clone(),
                    classified.status.as_str(),
                )
            })
            .collect()
    }

    /// The world this snapshot describes — one entry per classified candidate,
    /// carrying SC-017o's completeness fact forward.
    ///
    /// **Reads nothing.** Every SC-509 field comes from the record snapshot
    /// phase 1 captured at discovery ([`crate::session::RecordSnapshot`]), so the
    /// digest's record facts and the liveness printed beside them are one
    /// observation of the world rather than two.
    ///
    /// A tmux-only candidate has no directory to read, so it is SC-509b
    /// `degraded` by construction — SC-017j: "a positively live tmux-only
    /// candidate remains visible; loss of its durable record is the separate
    /// SC-509b `degraded` fact".
    #[must_use]
    pub fn world(&self, now: Timestamp, unanswered_secs: i64) -> World {
        let sessions = self
            .snapshot
            .sessions
            .iter()
            .map(|classified| match &classified.candidate.durable {
                Some(record) => crate::session::entry_from(
                    &record.snapshot,
                    &record.name,
                    &SessionRuntime::new(classified.status),
                    now,
                    unanswered_secs,
                ),
                None => SessionEntry::degraded(&classified.candidate.name, classified.status),
            })
            .collect();
        World {
            now,
            sessions,
            // SC-017o's fact crosses as a COUNT of distinct failed sources,
            // never as the paths — see [`World::losses`].
            losses: distinct_losses(&self.snapshot.incomplete),
        }
    }
}

/// How many DISTINCT logical sources failed.
///
/// Counted HERE, at the boundary, because presentation must not hold the loss
/// paths — see [`World::losses`]. Distinct by source key: the row's fact is how
/// many sources were lost, not how many records exist.
fn distinct_losses(losses: &[FailedSource]) -> usize {
    let mut seen: Vec<&FailedSource> = Vec::new();
    for loss in losses {
        if !seen.contains(&loss) {
            seen.push(loss);
        }
    }
    seen.len()
}

/// What `ae list` writes to STDERR for `world`, if anything.
///
/// **SC-017o** — a human listing keeps its partial table and says, explicitly,
/// that it could not look everywhere. The message carries at least the NUMBER of
/// failed logical sources, because the useful fact is not WHICH sessions were
/// lost — nobody can know that — but that absence in this snapshot is not proof.
///
/// Emitted for EVERY human view, not just `--all`: a warning that appears only
/// under one filter is a warning most invocations never see, and the filter a
/// human happened to type has nothing to do with whether ae managed to
/// enumerate. Wording, whether paths are named, and exit status are open
/// choices; the count is not.
///
/// `None` for a complete snapshot — silence is the correct answer when nothing
/// was lost, and a diagnostic that always fires stops carrying information.
#[must_use]
pub fn diagnostic(world: &World) -> Option<String> {
    let lost = world.loss_count();
    if lost == 0 {
        return None;
    }
    let sources = if lost == 1 { "source" } else { "sources" };
    Some(format!(
        "ae: warning: inventory incomplete — {lost} logical {sources} could not be enumerated; \
         sessions they may hold are absent from this listing"
    ))
}

/// What `ae list` writes to stdout for `args` over `world`.
///
/// The returned string is the complete payload including its final newline, or
/// empty when a tabular listing selected nothing. A `--json` listing is never
/// empty: SC-509's document exists whether or not any session survived the
/// filters.
///
/// ```
/// use ae::digest::{SessionEntry, Status};
/// use ae::filters::ListArgs;
/// use ae::listing::{World, render};
/// use ae::time::Timestamp;
///
/// let world = World::new(
///     Timestamp::from_epoch(0),
///     vec![
///         SessionEntry::new("live", Status::Running),
///         SessionEntry::new("old", Status::Stopped),
///     ],
/// );
///
/// // SC-017a: the default shows running sessions only — in both renderings.
/// let json = render(&ListArgs::parse(&["--json"])?, &world);
/// assert!(json.contains(r#""name":"live""#));
/// assert!(!json.contains(r#""name":"old""#));
/// assert!(render(&ListArgs::default(), &world).contains("live"));
/// # Ok::<(), ae::filters::UnknownFlag>(())
/// ```
#[must_use]
pub fn render(args: &ListArgs, world: &World) -> String {
    // SC-017f: ONE selection. Both arms below read this same answer.
    let selected = args.selection.select(&world.sessions, world.now);
    if args.json {
        let mut out = Digest::new(
            world.now,
            selected.into_iter().cloned().collect::<Vec<SessionEntry>>(),
            // Carried, never derived from the selection: a filter cannot make an
            // incomplete enumeration complete.
            world.inventory_complete(),
        )
        .render();
        // The newline is a stdout convention, not part of SC-509's object —
        // which is why it is added at the boundary and not inside `Digest`.
        out.push('\n');
        out
    } else {
        table(&selected)
    }
}

/// The tabular view — SC-017h's three nouns.
///
/// **The layout is PROVISIONAL and unratified** (see the module docs). What is
/// contractual is that each session line carries the session's `attn:<reason>`
/// marker when and only when it needs attention, and that each agent line
/// carries that agent's health and its declared state.
#[must_use]
pub fn table(sessions: &[&SessionEntry]) -> String {
    let mut out = String::new();
    for session in sessions {
        out.push_str(&session.name);
        out.push('\t');
        out.push_str(session.status.as_str());
        // SC-017g's rollup, "when a session needs attention" — so the marker is
        // absent rather than empty when nothing is wrong.
        if let Some(reason) = session.attention {
            out.push('\t');
            out.push_str("attn:");
            out.push_str(reason.as_str());
        }
        out.push('\n');
        for agent in &session.agents {
            out.push_str("  ");
            out.push_str(&agent.reference);
            out.push('\t');
            // Per-agent HEALTH. `dead` here is the boolean `alive: false` — a
            // pane fact — and is not SC-980's `attn:dead` alert, which reaches
            // the session marker above by way of SC-017g's rollup.
            // **SC-017r** — three distinguishable, non-silent renderings. The
            // words are an open choice; that `unknown` is recognizable AS
            // unknown rather than as absence or blank is not. Frozen bash
            // rendered a failed query exactly like a healthy agent (empty
            // marker) while its JSON called the same agent dead: two surfaces
            // collapsing one unknown in opposite directions.
            out.push_str(match agent.alive {
                Some(true) => "alive",
                Some(false) => "dead",
                None => "unknown",
            });
            out.push('\t');
            // The declared state, or a placeholder: an agent that has declared
            // nothing must not render as an agent whose state is the empty
            // string.
            out.push_str(agent.state.as_deref().unwrap_or("-"));
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{World, render, table};
    use crate::attention::Reason;
    use crate::digest::{AgentEntry, SessionEntry, Status};
    use crate::filters::{DEFAULT_ACTIVE_WINDOW_SECS, ListArgs};
    use crate::json;
    use crate::time::Timestamp;

    const NOW: Timestamp = Timestamp::from_epoch(1_780_000_000);

    fn args(flags: &[&str]) -> ListArgs {
        ListArgs::parse(flags).expect("documented flags")
    }

    fn agent(reference: &str, alive: Option<bool>, state: Option<&str>) -> AgentEntry {
        AgentEntry {
            reference: reference.to_owned(),
            alias: reference.split(':').next().unwrap_or("").to_owned(),
            name: reference.split(':').nth(1).unwrap_or("").to_owned(),
            session_id: None,
            alive,
            state: state.map(ToOwned::to_owned),
            reason: None,
        }
    }

    fn world() -> World {
        let mut live = SessionEntry::new("live", Status::Running);
        live.attention = Some(Reason::Blocked);
        live.last_active_epoch = Some(NOW.epoch() - 10);
        live.agents = vec![
            agent("claude:lead", Some(true), Some("blocked")),
            agent("codex:coworker", Some(false), None),
        ];

        let mut quiet = SessionEntry::new("quiet", Status::Running);
        quiet.agents = vec![agent("claude:solo", Some(true), Some("working"))];

        let old = SessionEntry::new("old", Status::Stopped);

        World::new(NOW, vec![old, live, quiet])
    }

    fn json_of(flags: &[&str]) -> json::Value {
        json::parse(&render(&args(flags), &world())).expect("one complete document")
    }

    fn names(value: &json::Value) -> Vec<String> {
        let Some(json::Value::Arr(sessions)) = value.get("sessions") else {
            panic!("sessions must be an array");
        };
        sessions
            .iter()
            .filter_map(|session| session.get_str("name"))
            .map(ToOwned::to_owned)
            .collect()
    }

    #[test]
    fn sc_509_the_json_rendering_is_the_digest_verbatim_plus_a_newline() {
        // Not a second shape: the bytes are `Digest::render` exactly, so a
        // change to SC-509's schema cannot be silently absorbed here.
        let rendered = render(&args(&["--json"]), &world());
        let expected = crate::digest::Digest::new(
            NOW,
            world()
                .sessions
                .into_iter()
                .filter(|session| session.status == Status::Running)
                .collect(),
            world().inventory_complete(),
        )
        .render();
        assert_eq!(rendered, format!("{expected}\n"));
    }

    #[test]
    fn sc_017a_the_default_json_listing_is_running_sessions_only() {
        assert_eq!(names(&json_of(&["--json"])), ["live", "quiet"]);
    }

    #[test]
    fn sc_017b_all_orders_running_sessions_before_stopped_ones() {
        assert_eq!(
            names(&json_of(&["--all", "--json"])),
            ["live", "quiet", "old"]
        );
    }

    #[test]
    fn sc_017c_stopped_shows_stopped_sessions_only() {
        assert_eq!(names(&json_of(&["--stopped", "--json"])), ["old"]);
    }

    #[test]
    fn sc_017d_needs_attn_and_every_alias_select_the_same_sessions() {
        for flag in ["--needs-attn", "--needs-me", "--needs", "--attn"] {
            assert_eq!(names(&json_of(&[flag, "--json"])), ["live"], "{flag}");
        }
    }

    #[test]
    fn sc_017e_active_and_its_alias_select_the_same_sessions() {
        for flag in ["--active", "--busy"] {
            assert_eq!(names(&json_of(&[flag, "--json"])), ["live"], "{flag}");
        }
    }

    #[test]
    fn sc_017i_running_renders_exactly_what_the_bare_default_renders() {
        assert_eq!(
            render(&args(&["--running", "--json"]), &world()),
            render(&args(&["--json"]), &world())
        );
        assert_eq!(
            render(&args(&["--running"]), &world()),
            render(&args(&[]), &world())
        );
    }

    #[test]
    fn sc_017f_json_honours_the_filters_and_never_widens_them() {
        // The row's actual content: --json is a RENDERING. For every filter, the
        // two renderings must cover the same sessions.
        //
        // Asserted through EXPECTED SELECTION DATA rather than by parsing rows:
        // the digest names the sessions it selected, and the tabular rendering
        // must equal the rendering of exactly those, in that order. Both sides
        // move together under any format change, so nothing here knows what
        // separates a column or where a line starts — and the claim is stronger
        // than the old per-name membership check, which could not see a session
        // rendered twice or two renderings that disagreed about order.
        let world = world();
        for flags in [
            vec![],
            vec!["--all"],
            vec!["--stopped"],
            vec!["--needs-attn"],
            vec!["--active"],
            vec!["--all", "--needs-attn"],
        ] {
            let mut json_flags = flags.clone();
            json_flags.push("--json");
            let selected = names(&json_of(&json_flags));
            let expected: Vec<&SessionEntry> = selected
                .iter()
                .map(|name| {
                    world
                        .sessions
                        .iter()
                        .find(|session| &session.name == name)
                        .unwrap_or_else(|| panic!("the digest named {name}, which no session has"))
                })
                .collect();
            assert_eq!(
                render(&args(&flags), &world),
                table(&expected),
                "{flags:?}: the two renderings do not cover the same sessions"
            );
        }
    }

    #[test]
    fn sc_521a_a_cross_dimension_combination_intersects_rather_than_erroring() {
        // --stopped --needs-attn selects nothing; --all --needs-attn keeps only
        // the matching RUNNING session. Neither is a usage error.
        assert!(names(&json_of(&["--stopped", "--needs-attn", "--json"])).is_empty());
        assert!(names(&json_of(&["--stopped", "--active", "--json"])).is_empty());
        assert_eq!(
            names(&json_of(&["--all", "--needs-attn", "--json"])),
            ["live"]
        );
        assert_eq!(names(&json_of(&["--all", "--active", "--json"])), ["live"]);
    }

    #[test]
    fn sc_521a_an_empty_intersection_is_still_a_complete_document() {
        // SC-506/SC-509: selecting nothing produces a document, not silence.
        let value = json_of(&["--stopped", "--needs-attn", "--json"]);
        assert_eq!(
            value.get("schema_version"),
            Some(&json::Value::Num(crate::digest::SCHEMA_VERSION))
        );
        assert_eq!(value.get("sessions"), Some(&json::Value::Arr(vec![])));
        assert_eq!(
            value.get("inventory_complete"),
            Some(&json::Value::Bool(true)),
            "SC-017o: an empty SELECTION is not an incomplete enumeration"
        );
    }

    #[test]
    fn sc_521b_the_last_distinct_scope_selector_decides_what_is_rendered() {
        // The parse rule is pinned in `filters`; what is pinned HERE is that the
        // rendering obeys it — the flags reach the output, not just the struct.
        assert_eq!(names(&json_of(&["--all", "--stopped", "--json"])), ["old"]);
        assert_eq!(
            names(&json_of(&["--stopped", "--all", "--json"])),
            ["live", "quiet", "old"]
        );
        assert_eq!(
            names(&json_of(&["--stopped", "--all", "--running", "--json"])),
            ["live", "quiet"]
        );
    }

    #[test]
    fn sc_521b_repeating_a_scope_flag_changes_no_byte_of_the_output() {
        for flag in ["--running", "--all", "--stopped"] {
            assert_eq!(
                render(&args(&[flag, "--json"]), &world()),
                render(&args(&[flag, flag, flag, "--json"]), &world()),
                "{flag}"
            );
        }
    }

    #[test]
    fn sc_017h_a_session_line_carries_the_attn_marker_only_when_it_needs_one() {
        // CONTENT, not layout — and proven through ISOLATED WORLDS rather than
        // by finding a session's line: each session is rendered ALONE, so
        // "carries the marker" needs no idea of where its line begins or what
        // separates its columns.
        let world = world();
        for session in &world.sessions {
            let alone = table(&[session]);
            match session.attention {
                Some(reason) => assert!(
                    alone.contains(&format!("attn:{}", reason.as_str())),
                    "{}: {alone}",
                    session.name
                ),
                None => assert!(
                    !alone.contains("attn:"),
                    "a session with nothing wrong carries no marker: {alone}"
                ),
            }
        }

        // And the marker survives the full listing: SC-017g's rollup reaches
        // exactly the sessions that need it, and no others.
        let flagged = world
            .sessions
            .iter()
            .filter(|session| session.attention.is_some())
            .count();
        assert_eq!(
            render(&args(&["--all"]), &world).matches("attn:").count(),
            flagged
        );
    }

    #[test]
    fn sc_017h_every_reason_reaches_the_marker_by_its_own_name() {
        for reason in Reason::BY_SEVERITY {
            let mut session = SessionEntry::new("s", Status::Running);
            session.attention = Some(reason);
            let rendered = table(&[&session]);
            assert!(
                rendered.contains(&format!("attn:{}", reason.as_str())),
                "{reason:?}: {rendered}"
            );
        }
    }

    #[test]
    fn sc_017h_the_roster_lists_every_agent_and_drops_none() {
        // Membership over the whole document. Whether each agent's OWN nouns
        // reach it is proven agent-by-agent in
        // `sc_017h_every_listed_session_brings_its_agents_health_and_declared_state`,
        // where isolation makes `contains` mean THAT agent. What is proven here
        // is that a multi-agent roster loses nobody — including the unhealthy
        // one, which is the agent a listing is most tempting to skip.
        let world = world();
        let rendered = table(&world.sessions.iter().collect::<Vec<_>>());
        for listed in world.sessions.iter().flat_map(|session| &session.agents) {
            assert!(
                rendered.contains(&listed.reference),
                "{} is missing from the roster: {rendered}",
                listed.reference
            );
        }
        assert!(rendered.contains("dead"), "{rendered}");
        assert!(rendered.contains("alive"), "{rendered}");
    }

    #[test]
    fn sc_017h_an_agent_that_declared_nothing_is_not_rendered_as_blank() {
        // CONTENT, not form. Contractual: "declared nothing" is a visible
        // answer, and it is not the same answer as "declared the empty string".
        // WHICH glyph stands in, and what separates the fields, is provisional
        // layout (see the module docs) and stays unpinned here.
        // An ISOLATED WORLD — one session, one agent — so the whole rendering
        // can be reasoned about without picking a line out of it.
        let rendered_with = |state: Option<&str>| {
            let mut session = SessionEntry::new("solo-session", Status::Running);
            session.agents = vec![agent("claude:lead", Some(true), state)];
            table(&[&session])
        };

        // Everything the rendering is REQUIRED to carry, taken away: whatever
        // is left is where the state went, and it has to be something.
        let undeclared = rendered_with(None);
        let residue = undeclared
            .replacen("solo-session", "", 1)
            .replacen(Status::Running.as_str(), "", 1)
            .replacen("claude:lead", "", 1)
            .replacen("alive", "", 1);
        assert!(
            residue.chars().any(|glyph| !glyph.is_whitespace()),
            "an undeclared state rendered as nothing at all: {undeclared:?}"
        );
        assert_ne!(
            undeclared,
            rendered_with(Some("")),
            "an agent that declared nothing renders as one that declared emptiness"
        );
        assert_ne!(
            undeclared,
            rendered_with(Some("working")),
            "and it does not render as one that declared a state"
        );
    }

    #[test]
    fn sc_017h_the_agent_level_reason_is_not_the_session_marker() {
        // SC-017h names three nouns and `attn:` is the SESSION rollup, so an
        // agent's OWN reason is not rendered at all.
        //
        // Tested as that semantic rule rather than by finding the agent's line:
        // toggling the reason through every value must change NO byte of the
        // rendering, and the session's single marker must survive. Asking "is
        // there an `attn:` on the agent's line" assumes the agent HAS a line —
        // a layout fact, and one a two-line card would break while changing
        // nothing about the rule.
        let mut session = SessionEntry::new("s", Status::Running);
        session.attention = Some(Reason::Dead);
        session.agents = vec![agent("claude:lead", Some(false), Some("working"))];

        let unflagged = table(&[&session]);
        for reason in Reason::BY_SEVERITY {
            let mut flagged = session.clone();
            flagged.agents[0].reason = Some(reason);
            assert_eq!(
                table(&[&flagged]),
                unflagged,
                "{reason:?} reached the rendering"
            );
        }
        assert_eq!(
            unflagged.matches("attn:").count(),
            1,
            "exactly one marker, the session's: {unflagged}"
        );
    }

    #[test]
    fn a_tabular_listing_that_selected_nothing_carries_no_session() {
        // Structural rather than byte-exact: a selection that matched nothing
        // renders exactly as a world containing nothing does — no session leaks
        // through the filter, whatever an empty listing eventually looks like.
        // The empty-listing TEXT is provisional layout and stays unpinned.
        let nothing_selected = render(&args(&["--stopped", "--needs-attn"]), &world());
        let nothing_exists = render(&args(&[]), &World::new(NOW, Vec::new()));
        assert_eq!(nothing_selected, nothing_exists, "{nothing_selected:?}");
        for name in ["live", "quiet", "old"] {
            assert!(
                !nothing_selected.contains(name),
                "{name} survived the filters: {nothing_selected:?}"
            );
        }
        // The JSON rendering is the opposite, and IS ratified: SC-506/SC-509's
        // document exists whether or not anything survived.
        assert!(!render(&args(&["--stopped", "--needs-attn", "--json"]), &world()).is_empty());
    }

    #[test]
    fn sc_017h_every_listed_session_brings_its_agents_health_and_declared_state() {
        // ISOLATED WORLDS, one agent at a time. What is asserted is that the
        // agent's nouns are THERE — never which line they landed on, what
        // separates them, or what order the columns come in. Rendering the agent
        // ALONE is what lets `contains` mean "this agent's health" instead of
        // "somebody's": with two agents in the tree, `alive` is in the document
        // either way, and that is exactly what row-parsing used to buy.
        //
        // Which sessions are listed under which filter is a different claim, and
        // `sc_017f_json_honours_the_filters_and_never_widens_them` carries it.
        let world = world();
        for session in &world.sessions {
            for listed in &session.agents {
                let mut solo = session.clone();
                solo.agents = vec![listed.clone()];
                let alone = table(&[&solo]);

                // The negative half depends on the fixture's words, not on the
                // layout: no name, status or state here spells the other health.
                let (health, other) = match listed.alive {
                    Some(true) => ("alive", "dead"),
                    Some(false) => ("dead", "alive"),
                    None => ("unknown", "dead"),
                };
                assert!(alone.contains(&listed.reference), "{alone}");
                assert!(alone.contains(health), "{alone}");
                assert!(
                    !alone.contains(other),
                    "the health word is this agent's own: {alone}"
                );
                if let Some(declared) = listed.state.as_deref() {
                    assert!(alone.contains(declared), "{alone}");
                }
            }
        }
    }

    #[test]
    fn sc_017e_the_window_is_the_one_the_flag_selected() {
        // Guards the seam between the flag's default and the selection: a
        // session just outside the window is not listed by --active.
        let mut stale = SessionEntry::new("stale", Status::Running);
        stale.last_active_epoch = Some(NOW.epoch() - DEFAULT_ACTIVE_WINDOW_SECS - 1);
        let world = World::new(NOW, vec![stale]);
        assert!(
            !render(&args(&["--active"]), &world).contains("stale"),
            "a session outside the window is not listed by --active"
        );
        assert!(render(&args(&[]), &world).contains("stale"));
    }

    #[test]
    fn sc_509_a_world_with_no_sessions_still_renders_a_complete_document() {
        // Only the ratified half is pinned. What the TABULAR rendering prints
        // for a world with nothing in it is provisional layout; that it carries
        // no session is asserted structurally in
        // `a_tabular_listing_that_selected_nothing_carries_no_session`.
        let empty = World::new(NOW, Vec::new());
        assert_eq!(
            render(&args(&["--json"]), &empty),
            format!(
                "{{\"schema_version\":2,\"generated_at\":\"{NOW}\",\"sessions\":[],\"inventory_complete\":true}}\n"
            )
        );
    }
}
