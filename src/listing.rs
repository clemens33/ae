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
//! columns, no field order and no widths. Ratified with the seats for this
//! slice:
//!
//! * what [`table`] SHOWS is pinned — the row's three nouns plus frozen's
//!   per-session goal/git/version/activity subline;
//! * what it LOOKS LIKE is **provisional and unratified** — only layout bytes
//!   remain open; capture-backed residual bytes are not layout.
//!
//! Two things follow, and both are deliberate. An agent's own attention reason
//! is NOT rendered: SC-017h names three nouns and `attn:` is the SESSION-level
//! marker (SC-017g's rollup). Frozen's three established empty-listing messages
//! are retained; no message is invented for the remaining empty scopes.

use crate::digest::{Digest, SessionEntry};
use crate::filters::{ListArgs, Scope};
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
/// The returned string is the complete payload including its final newline. A
/// tabular listing retains frozen's established empty-state messages for every
/// scope and filter combination.
/// A `--json` listing is never empty: SC-509's document exists whether or not
/// any session survived the filters.
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
    } else if selected.is_empty() {
        frozen_empty_listing(args).to_owned()
    } else {
        table_at(&selected, world.now)
    }
}

/// Frozen's three observed messages for an empty human selection.
///
/// Attention deliberately wins over activity, matching the predecessor's
/// ordered checks when both filters were named. Frozen `ae:4313-4321` is the
/// authority for the two source-derived messages whose empty states have no
/// capture oracle in the phase-4 corpus.
const fn frozen_empty_listing(args: &ListArgs) -> &'static str {
    if args.selection.needs_attention {
        "No running sessions need your attention.\n"
    } else if args.selection.active_within_secs.is_some() {
        "No recently active sessions.\n"
    } else if matches!(args.selection.scope, Scope::Running) {
        "No running ae sessions. (try: ae list --all)\n"
    } else if matches!(args.selection.scope, Scope::All) {
        "No ae sessions.\n"
    } else {
        "No stopped ae sessions.\n"
    }
}

/// The tabular view — SC-017h's three nouns.
///
/// **The layout is PROVISIONAL and unratified** (see the module docs). What is
/// contractual is that an exact session maximum carries `attn:<reason>`.
/// `--needs-attn` may still select a row on its partial-evidence `true`, but human
/// output never fabricates an inexact class. Each agent line carries that
/// agent's health and its declared state.
#[must_use]
pub fn table(sessions: &[&SessionEntry]) -> String {
    // `render` is the product route and supplies its snapshot time to
    // `table_at`. This compatibility entry point preserves its deterministic
    // epoch-zero clock: nonzero activity timestamps therefore have frozen's
    // future-time spelling, `just now`, rather than an ambient-clock result.
    table_at(sessions, Timestamp::from_epoch(0))
}

/// The tabular view at the snapshot time that supplied `sessions`.
///
/// Frozen bash's subline was clock-relative, so the production route must pass
/// the same snapshot clock that selected the sessions. The header, columns and
/// agent-row layout remain provisional; the subline is residual phase-4 output.
#[must_use]
pub fn table_at(sessions: &[&SessionEntry], now: Timestamp) -> String {
    let mut out = String::new();
    for session in sessions {
        out.push_str(&session.name);
        out.push('\t');
        out.push_str(session.status.as_str());
        // The filter may have selected this row on readable partial evidence, but
        // a table marker names a class and therefore needs the same exactness as
        // JSON's attention/rank pair.
        if session.attention_is_exact()
            && let Some(reason) = session.attention
        {
            out.push('\t');
            out.push_str("attn:");
            out.push_str(reason.as_str());
        }
        out.push('\n');
        push_frozen_session_subline(&mut out, session, now);
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
            // SC-017h's amended declared-state cell is three-way: an exact
            // declaration, an exact no-declaration, or unreadable event input.
            // The last spelling cannot reuse `-`, which means the reader did
            // establish there was no declaration.
            out.push_str(
                match (session.agent_state_is_exact(), agent.state.as_deref()) {
                    (true, Some(state)) => state,
                    (true, None) => "-",
                    (false, _) => "unknown",
                },
            );
            out.push('\n');
        }
    }
    out
}

/// Append the retained, frozen-bash session summary.
///
/// The current table carries the session attention marker on its semantic
/// session row. It intentionally does not repeat that marker here: phase 4
/// classifies the frozen marker as semantic while this whole subline is
/// residual, so duplicating it would turn a retained semantic fact into a
/// residual divergence.
fn push_frozen_session_subline(out: &mut String, session: &SessionEntry, now: Timestamp) {
    out.push_str("  ");
    if let Some(goal) = session.goal.as_deref().filter(|goal| !goal.is_empty()) {
        out.push_str("goal");
        if let Some(goal_set_epoch) = session.goal_set_epoch.filter(|epoch| *epoch > 0) {
            out.push_str(" (");
            out.push_str(&frozen_relative_time(now, Some(goal_set_epoch)));
            out.push(')');
        }
        out.push_str(": ");
        push_frozen_goal(out, goal);
        out.push_str(" · ");
    }
    if let Some(branch) = session
        .branch
        .as_deref()
        .filter(|branch| !branch.is_empty())
    {
        out.push_str("git:");
        out.push_str(branch);
        out.push_str(" · ");
    } else if !session.degraded {
        // SC-405g's temporary predecessor projection. Branch acquisition has
        // not landed yet, so healthy `None` is a value placeholder rather than
        // a missing atom; remove this arm with that acquisition slice.
        out.push_str("git:? · ");
    }
    out.push_str("ae ");
    out.push_str(
        session
            .ae_version
            .as_deref()
            .filter(|version| !version.is_empty())
            .unwrap_or("?"),
    );
    out.push_str(" · active ");
    out.push_str(&frozen_relative_time(now, session.last_active_epoch));
    out.push('\n');
}

/// Frozen bash keeps at most 60 characters of a goal, reserving its final
/// character for an ellipsis when truncation happens.
fn push_frozen_goal(out: &mut String, goal: &str) {
    if goal.chars().count() > 60 {
        out.extend(goal.chars().take(59));
        out.push('…');
    } else {
        out.push_str(goal);
    }
}

/// Frozen `format_relative_time`, evaluated at the listing snapshot.
fn frozen_relative_time(now: Timestamp, timestamp: Option<i64>) -> String {
    let Some(timestamp) = timestamp.filter(|timestamp| *timestamp > 0) else {
        return "-".to_owned();
    };
    let delta = now.epoch().saturating_sub(timestamp);
    if delta < 0 {
        "just now".to_owned()
    } else if delta < 60 {
        format!("{delta}s ago")
    } else if delta < 3_600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86_400 {
        format!("{}h ago", delta / 3_600)
    } else if delta < 604_800 {
        format!("{}d ago", delta / 86_400)
    } else {
        ">7d".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{Presentation, World, render, table};
    use crate::attention::Reason;
    use crate::digest::{AgentEntry, SessionEntry, Status};
    use crate::filters::{DEFAULT_ACTIVE_WINDOW_SECS, ListArgs};
    use crate::inventory::{Candidate, DurableRecord, Layout};
    use crate::json;
    use crate::liveness::{Classified, Snapshot};
    use crate::meta::ServerSelector;
    use crate::session::{
        DEFAULT_UNANSWERED_SECS, MetaRead, RecordSnapshot, SessionRuntime, entry_for,
    };
    use crate::time::Timestamp;

    const NOW: Timestamp = Timestamp::from_epoch(1_780_000_000);
    const GOAL_BRANCH_ACTIVE_CAPTURE: &str = include_str!(
        "../docs/migration/evidence/batch-c-artifacts/arms/A1/c01-healthy-ro/out/list-all.stdout"
    );
    const NO_GOAL_CAPTURE: &str = include_str!(
        "../docs/migration/evidence/batch-c-artifacts/arms/A1/c04-empty-vs-omitted-ro/out/list-all.stdout"
    );
    const DEGRADED_META_CAPTURE: &str = include_str!(
        "../docs/migration/evidence/batch-c-artifacts/arms/A1/c02-meta-mode-000-ro/out/list-all.stdout"
    );
    const NO_RUNNING_CAPTURE: &str = include_str!(
        "../docs/migration/evidence/batch-c-artifacts/arms/A1/c01-healthy-ro/out/list.stdout"
    );
    const NO_ACTIVE_CAPTURE: &str = include_str!(
        "../docs/migration/evidence/batch-c-artifacts/arms/A2/c01-filters-ro/out/list_active.stdout"
    );
    const NO_NEEDS_ATTENTION_CAPTURE: &str = include_str!(
        "../docs/migration/evidence/batch-c-artifacts/arms/A2/c01-filters-ro/out/win_inside_active_needsattn.stdout"
    );

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

    /// The one residual line after the session row, including its newline.
    fn successor_subline_bytes(rendered: &str) -> &[u8] {
        let (_, after_session) = rendered
            .split_once('\n')
            .unwrap_or_else(|| panic!("a selected session row: {rendered:?}"));
        let end = after_session
            .find('\n')
            .unwrap_or_else(|| panic!("the session subline must end: {rendered:?}"));
        &after_session.as_bytes()[..=end]
    }

    /// The matching frozen residual line after its header and session row.
    fn frozen_capture_subline_bytes(capture: &str) -> &[u8] {
        let (_, after_header) = capture
            .split_once('\n')
            .unwrap_or_else(|| panic!("a frozen header: {capture:?}"));
        let (_, after_session) = after_header
            .split_once('\n')
            .unwrap_or_else(|| panic!("a frozen session row: {capture:?}"));
        let end = after_session
            .find('\n')
            .unwrap_or_else(|| panic!("a frozen subline: {capture:?}"));
        &after_session.as_bytes()[..=end]
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

    #[test]
    fn formatter_goal_branch_version_and_active_match_the_frozen_capture_residual() {
        // Fixed `World::now` exercises the relative-text FORMATTER only. It is
        // not a claim that a later replay has the capture's clock.
        let mut entry = SessionEntry::new("tg1", Status::Stopped);
        entry.goal = Some("healthy fixture session goal".to_owned());
        entry.goal_set_epoch = Some(NOW.epoch() - 1_440);
        entry.branch = Some("master".to_owned());
        entry.ae_version = Some("0.2.1".to_owned());
        entry.last_active_epoch = Some(NOW.epoch() - 1_440);
        let world = World::new(NOW, vec![entry]);

        assert_eq!(
            successor_subline_bytes(&render(&args(&["--all"]), &world)),
            frozen_capture_subline_bytes(GOAL_BRANCH_ACTIVE_CAPTURE)
        );
    }

    #[test]
    fn formatter_null_goal_uses_the_frozen_no_goal_capture_shape() {
        // Fixed `World::now` exercises the relative-text FORMATTER only. It is
        // not a claim that a later replay has the capture's clock.
        let mut entry = SessionEntry::new("ta1b", Status::Stopped);
        entry.branch = Some("master".to_owned());
        entry.ae_version = Some("0.2.1".to_owned());
        entry.last_active_epoch = Some(NOW.epoch() - 1_380);
        let world = World::new(NOW, vec![entry]);

        assert_eq!(
            successor_subline_bytes(&render(&args(&["--all"]), &world)),
            frozen_capture_subline_bytes(NO_GOAL_CAPTURE)
        );
    }

    #[test]
    fn formatter_degraded_meta_uses_the_frozen_placeholder_capture_shape() {
        // Fixed `World::now` exercises the relative-text FORMATTER only. It is
        // not a claim that a later replay has the capture's clock.
        let mut entry = SessionEntry::degraded("tg1", Status::Stopped);
        entry.last_active_epoch = Some(NOW.epoch() - 1_440);
        let world = World::new(NOW, vec![entry]);

        assert_eq!(
            successor_subline_bytes(&render(&args(&["--all"]), &world)),
            frozen_capture_subline_bytes(DEGRADED_META_CAPTURE)
        );
    }

    #[test]
    fn formatter_goal_truncation_counts_characters_at_the_frozen_boundaries() {
        // A 60-character goal with one two-byte character proves this is a
        // character limit, not a UTF-8-byte limit. Frozen ae:3272 leaves it
        // intact; at 61 characters it keeps 59 and appends an ellipsis.
        let exactly_sixty = format!("{}é", "a".repeat(59));
        let sixty_one = format!("{exactly_sixty}z");
        let render_goal = |goal: String| {
            let mut entry = SessionEntry::new("long-goal", Status::Running);
            entry.goal = Some(goal);
            entry.ae_version = Some("0.2.1".to_owned());
            let world = World::new(NOW, vec![entry]);
            String::from_utf8(successor_subline_bytes(&render(&args(&[]), &world)).to_vec())
                .expect("the human output is UTF-8")
        };

        assert_eq!(
            render_goal(exactly_sixty.clone()),
            format!("  goal: {exactly_sixty} · git:? · ae 0.2.1 · active -\n")
        );
        assert_eq!(
            render_goal(sixty_one),
            format!(
                "  goal: {}… · git:? · ae 0.2.1 · active -\n",
                "a".repeat(59)
            )
        );
    }

    #[test]
    fn formatter_sc_405g_temporary_unobserved_branch_keeps_the_git_atom() {
        // Branch acquisition has not landed. This temporary placeholder arm
        // keeps a healthy row's atom present until that source exists.
        let mut entry = SessionEntry::new("no-branch", Status::Running);
        entry.ae_version = Some("0.2.1".to_owned());
        entry.last_active_epoch = Some(NOW.epoch() - 1_380);
        let world = World::new(NOW, vec![entry]);
        let rendered = render(&args(&[]), &world);
        let subline = successor_subline_bytes(&rendered);

        assert_eq!(
            subline,
            b"  git:? \xC2\xB7 ae 0.2.1 \xC2\xB7 active 23m ago\n"
        );
    }

    #[test]
    fn formatter_sc_405g_degraded_unobserved_branch_omits_the_git_atom() {
        // Meta is readable in both cases, so the retained version remains
        // visible. Their independent loss facts make the entry degraded and
        // therefore retain frozen's no-branch placeholder shape.
        let event_loss_fixture = DigestFixture::new(
            "subline-event-loss",
            Some("mode=local\nae_version=0.2.1\n"),
            Some("not an event\n"),
        );
        let event_loss = entry_for(
            &event_loss_fixture.0,
            "event-loss",
            &SessionRuntime::new(Status::Running),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        let duplicate_goal_fixture = DigestFixture::new(
            "subline-duplicate-goal",
            Some("mode=local\ngoal=first\ngoal=second\nae_version=0.2.1\n"),
            None,
        );
        let duplicate_goal = entry_for(
            &duplicate_goal_fixture.0,
            "duplicate-goal",
            &SessionRuntime::new(Status::Running),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );

        for entry in [event_loss, duplicate_goal] {
            assert!(entry.degraded);
            let world = World::new(NOW, vec![entry]);
            assert_eq!(
                successor_subline_bytes(&render(&args(&[]), &world)),
                b"  ae 0.2.1 \xC2\xB7 active -\n"
            );
        }
    }

    #[test]
    fn formatter_relative_time_matches_frozen_thresholds_at_a_fixed_snapshot() {
        for (age, expected) in [
            (-1, "just now"),
            (0, "0s ago"),
            (59, "59s ago"),
            (60, "1m ago"),
            (3_599, "59m ago"),
            (3_600, "1h ago"),
            (86_399, "23h ago"),
            (86_400, "1d ago"),
            (604_799, "6d ago"),
            (604_800, ">7d"),
        ] {
            assert_eq!(
                super::frozen_relative_time(NOW, Some(NOW.epoch() - age)),
                expected,
                "age {age}"
            );
        }
        for missing in [None, Some(0), Some(-1)] {
            assert_eq!(super::frozen_relative_time(NOW, missing), "-");
        }
    }

    #[test]
    fn frozen_empty_listing_messages_match_the_capture_bytes() {
        let empty = World::new(NOW, Vec::new());
        assert_eq!(render(&args(&[]), &empty), NO_RUNNING_CAPTURE);
        assert_eq!(render(&args(&["--active"]), &empty), NO_ACTIVE_CAPTURE);
        assert_eq!(
            render(&args(&["--needs-attn"]), &empty),
            NO_NEEDS_ATTENTION_CAPTURE
        );
        assert_eq!(
            render(&args(&["--active", "--needs-attn"]), &empty),
            NO_NEEDS_ATTENTION_CAPTURE,
            "frozen gives attention priority when both filters are named"
        );
    }

    #[test]
    fn source_derived_no_capture_oracle_empty_all_and_stopped_messages_match_frozen() {
        // The phase-4 corpus has no empty `--all` or `--stopped` invocation.
        // Frozen ae:4316-4321 is therefore the byte authority for both values.
        let empty = World::new(NOW, Vec::new());
        assert_eq!(render(&args(&["--all"]), &empty), "No ae sessions.\n");
        assert_eq!(
            render(&args(&["--stopped"]), &empty),
            "No stopped ae sessions.\n"
        );
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
                super::table_at(&expected, world.now),
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
        // The exact empty-state bytes have their capture pin above. This keeps
        // the selection claim separate: a message must not leak a filtered
        // session back into the human output.
        let nothing_selected = render(&args(&["--stopped", "--needs-attn"]), &world());
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
        // Only the ratified half is pinned: the member set of a complete empty
        // digest. Rendered member order is an open choice. The human's three
        // established empty states are pinned separately from this document.
        let empty = World::new(NOW, Vec::new());
        let rendered = render(&args(&["--json"]), &empty);
        let actual = json::parse(rendered.trim_end()).expect("one complete document");
        let expected = json::parse(&format!(
            r#"{{"schema_version":2,"generated_at":"{NOW}","sessions":[],"inventory_complete":true}}"#
        ))
        .expect("the expected bag is json");
        assert!(
            actual.same_members(&expected),
            "complete empty digest members: {rendered}"
        );
    }

    // ---- SC-509b: source knowledge survives aggregate degradation ----------

    /// One durable record projected through the real presentation boundary.
    ///
    /// The fixture writes only phase-1 inputs. Its tests never manufacture a
    /// `SessionEntry`, because the point is the provenance attached while
    /// projecting a captured record snapshot.
    struct DigestFixture(std::path::PathBuf);

    impl DigestFixture {
        fn new(tag: &str, meta: Option<&str>, events: Option<&str>) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("ae-listing-sc509b-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch directory");
            if let Some(meta) = meta {
                std::fs::write(dir.join("meta"), meta).expect("a meta fixture");
            }
            if let Some(events) = events {
                std::fs::write(dir.join("events.jsonl"), events).expect("an events fixture");
            }
            Self(dir)
        }

        fn snapshot(&self, name: &str) -> Snapshot {
            let record = RecordSnapshot::read(&self.0);
            Snapshot {
                sessions: vec![Classified {
                    candidate: Candidate {
                        name: name.to_owned(),
                        durable: Some(DurableRecord {
                            path: self.0.clone(),
                            name: name.to_owned(),
                            layout: Layout::Canonical,
                            server: ServerSelector::Missing,
                            meta_read: record.meta_read,
                            snapshot: record,
                        }),
                        live: None,
                    },
                    status: Status::Running,
                }],
                incomplete: Vec::new(),
            }
        }

        fn entry(&self, name: &str) -> json::Value {
            let world =
                Presentation::enter(&self.snapshot(name)).world(NOW, DEFAULT_UNANSWERED_SECS);
            let rendered = render(&args(&["--all", "--json"]), &world);
            let document = json::parse(rendered.trim_end()).expect("one digest");
            let Some(json::Value::Arr(sessions)) = document.get("sessions") else {
                panic!("sessions must be an array");
            };
            sessions
                .first()
                .cloned()
                .expect("the durable candidate remains")
        }

        fn entry_via_entry_for(&self, name: &str) -> json::Value {
            entry_for(
                &self.0,
                name,
                &SessionRuntime::new(Status::Running),
                NOW,
                DEFAULT_UNANSWERED_SECS,
            )
            .to_json()
        }
    }

    impl Drop for DigestFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sc_509b_event_loss_keeps_false_as_partial_evidence_and_omits_only_event_facts() {
        let fixture = DigestFixture::new(
            "event-loss",
            Some("mode=local\norigin=/repo\nagent.main=claude:lead\n"),
            Some("not json\n"),
        );
        let entry = fixture.entry("event-loss");

        assert_eq!(entry.get("degraded"), Some(&json::Value::Bool(true)));
        assert_eq!(entry.get_str("mode"), Some("local"));
        assert_eq!(entry.get_str("origin"), Some("/repo"));
        assert_eq!(entry.get("goal"), Some(&json::Value::Null));
        assert_eq!(entry.get("goal_set_epoch"), None);
        assert_eq!(entry.get("last_active_epoch"), None);

        // No readable contribution currently establishes attention. Under loss,
        // false is not quiet proof, so there is no null/zero pair.
        assert_eq!(
            entry.get("needs_attention"),
            Some(&json::Value::Bool(false))
        );
        assert_eq!(entry.get("attention"), None);
        assert_eq!(entry.get("attention_rank"), None);

        let Some(json::Value::Arr(agents)) = entry.get("agents") else {
            panic!("the readable roster remains an array");
        };
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].get("state"), None);
        assert_eq!(agents[0].get("reason"), None);
    }

    #[test]
    fn sc_509b_a_listed_agent_needs_complete_reason_inputs() {
        let fixture = DigestFixture::new(
            "reason-loss",
            Some("mode=local\nagent.main=claude:lead\n"),
            Some(concat!(
                r#"{"ts":"2026-05-29T14:00:00Z","actor":"claude:lead","action":"state","ref":"blocked"}"#,
                "\nnot json\n",
            )),
        );
        let entry = fixture.entry("reason-loss");

        // A skipped event could change this agent's current state and reason,
        // so partial evidence remains while current event-derived members omit.
        assert_eq!(entry.get("needs_attention"), Some(&json::Value::Bool(true)));
        assert_eq!(entry.get("attention"), None);
        assert_eq!(entry.get("attention_rank"), None);
        let Some(json::Value::Arr(agents)) = entry.get("agents") else {
            panic!("the readable agent remains visible");
        };
        assert_eq!(agents[0].get_str("ref"), Some("claude:lead"));
        assert_eq!(agents[0].get("state"), None);
        assert_eq!(agents[0].get("reason"), None);
    }

    #[test]
    fn sc_509b_entry_for_and_presentation_render_one_snapshot_identically() {
        let fixture = DigestFixture::new(
            "one-producer",
            Some("mode=local\nagent.main=claude:lead\n"),
            Some(concat!(
                r#"{"ts":"2026-05-29T14:00:00Z","actor":"claude:lead","action":"state","ref":"blocked"}"#,
                "\nnot json\n",
            )),
        );

        assert_eq!(
            fixture.entry("one-producer"),
            fixture.entry_via_entry_for("one-producer"),
            "the public producer, not presentation, owns provenance"
        );
    }

    #[test]
    fn sc_509b_needs_attention_selects_partial_evidence_without_printing_an_inexact_class() {
        let fixture = DigestFixture::new(
            "partial-evidence-table",
            Some("mode=local\nagent.main=claude:lead\n"),
            Some(concat!(
                r#"{"ts":"2026-05-29T14:00:00Z","actor":"claude:lead","action":"state","ref":"blocked"}"#,
                "\nnot json\n",
            )),
        );
        let world = Presentation::enter(&fixture.snapshot("partial-evidence-table"))
            .world(NOW, DEFAULT_UNANSWERED_SECS);
        let human = render(&args(&["--needs-attn"]), &world);
        let machine = render(&args(&["--needs-attn", "--json"]), &world);
        let digest = json::parse(machine.trim_end()).expect("one digest");
        let Some(json::Value::Arr(sessions)) = digest.get("sessions") else {
            panic!("one selected partial-evidence row");
        };

        assert!(human.contains("partial-evidence-table\trunning"));
        assert!(!human.contains("attn:blocked"));
        assert!(
            human.contains("  claude:lead\tunknown\tunknown\n"),
            "the malformed event hides stale blocked state behind the human unknown: {human}"
        );
        assert!(!human.contains("\tblocked\n"));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].get("needs_attention"),
            Some(&json::Value::Bool(true))
        );
        assert_eq!(sessions[0].get("attention"), None);
        let Some(json::Value::Arr(agents)) = sessions[0].get("agents") else {
            panic!("the readable roster remains an array");
        };
        assert_eq!(agents[0].get("state"), None);
    }

    #[test]
    fn sc_017h_declared_state_cell_distinguishes_exact_value_and_no_declaration() {
        let mut session = SessionEntry::new("state-cell", Status::Running);
        session.agents = vec![AgentEntry {
            reference: "claude:lead".to_owned(),
            alias: "claude".to_owned(),
            name: "lead".to_owned(),
            session_id: None,
            alive: Some(true),
            state: Some("blocked".to_owned()),
            reason: None,
        }];
        assert!(table(&[&session]).contains("  claude:lead\talive\tblocked\n"));

        session.agents[0].state = None;
        assert!(table(&[&session]).contains("  claude:lead\talive\t-\n"));
    }

    #[test]
    fn sc_509b_ledger_dead_with_event_loss_is_partial_for_session_and_agent() {
        let fixture = DigestFixture::new(
            "dead-dominates",
            Some("mode=local\nagent.main=claude:lead\n"),
            Some(concat!(
                r#"{"ts":"2026-05-29T14:00:00Z","actor":"_watchdog","action":"alert","target":"claude:lead","summary":"agent process dead — dropped to shell"}"#,
                "\nnot json\n",
            )),
        );
        let entry = fixture.entry("dead-dominates");

        assert_eq!(entry.get("degraded"), Some(&json::Value::Bool(true)));
        assert_eq!(entry.get("needs_attention"), Some(&json::Value::Bool(true)));
        assert_eq!(entry.get("attention"), None);
        assert_eq!(entry.get("attention_rank"), None);
        let Some(json::Value::Arr(agents)) = entry.get("agents") else {
            panic!("the readable roster remains an array");
        };
        assert_eq!(agents[0].get("reason"), None);
        let world = Presentation::enter(&fixture.snapshot("dead-dominates"))
            .world(NOW, DEFAULT_UNANSWERED_SECS);
        assert!(
            !render(&args(&["--all"]), &world).contains("attn:"),
            "partial ledger evidence may select a row but may not name an inexact class"
        );
    }

    #[test]
    fn sc_509b_a_readable_empty_roster_is_exact_and_not_degraded() {
        let fixture = DigestFixture::new("empty-roster", Some("mode=local\n"), Some(""));
        let entry = fixture.entry("empty-roster");

        assert_eq!(entry.get("degraded"), None);
        assert_eq!(entry.get("agents"), Some(&json::Value::Arr(vec![])));
        assert_eq!(
            entry.get("needs_attention"),
            Some(&json::Value::Bool(false))
        );
        assert_eq!(entry.get("attention"), Some(&json::Value::Null));
        assert_eq!(entry.get("attention_rank"), Some(&json::Value::Num(0)));
    }

    #[test]
    fn sc_509b_unrelated_meta_loss_keeps_an_exact_attention_maximum() {
        let fixture = DigestFixture::new(
            "unrelated-meta-loss",
            Some(
                "mode=local\nmode=duplicate\norigin=/repo\nwork_dir=/work\nagent.main=claude:lead\n",
            ),
            Some(concat!(
                r#"{"ts":"2026-05-29T14:00:00Z","actor":"claude:lead","action":"state","ref":"blocked"}"#,
                "\n",
            )),
        );
        let entry = fixture.entry("unrelated-meta-loss");

        // The duplicate makes only `mode` unreadable. The complete roster and
        // event stream still settle the session maximum exactly.
        assert_eq!(entry.get("degraded"), Some(&json::Value::Bool(true)));
        assert_eq!(entry.get("mode"), None);
        assert_eq!(entry.get_str("origin"), Some("/repo"));
        assert_eq!(entry.get_str("work_dir"), Some("/work"));
        assert_eq!(
            entry.get("goal"),
            Some(&json::Value::Null),
            "the absent goal is independently established despite duplicate mode"
        );
        assert_eq!(entry.get("needs_attention"), Some(&json::Value::Bool(true)));
        assert_eq!(entry.get_str("attention"), Some("blocked"));
        assert_eq!(
            entry.get("attention_rank"),
            Some(&json::Value::Num(Reason::Blocked.rank()))
        );
    }

    #[test]
    fn sc_509b_lost_roster_and_lost_events_omit_different_members() {
        let events_lost = DigestFixture::new(
            "events-separated",
            Some("mode=local\nagent.main=claude:lead\n"),
            Some("not json\n"),
        )
        .entry("events-separated");
        let roster_lost = DigestFixture::new(
            "roster-separated",
            Some("mode=local\nagent.main=claude:lead\nagent.worker.0=broken\n"),
            Some(concat!(
                r#"{"ts":"2026-05-29T14:00:00Z","actor":"claude:lead","action":"state","ref":"blocked"}"#,
                "\n",
            )),
        )
        .entry("roster-separated");

        assert_eq!(events_lost.get_str("mode"), Some("local"));
        assert_eq!(events_lost.get("last_active_epoch"), None);
        assert_eq!(roster_lost.get_str("mode"), Some("local"));
        assert_eq!(
            roster_lost.get("last_active_epoch"),
            Some(&json::Value::Num(1_780_063_200)),
            "the complete event stream remains known despite roster loss"
        );
        assert_eq!(roster_lost.get("attention"), None);
        assert_eq!(roster_lost.get("attention_rank"), None);
        let Some(json::Value::Arr(agents)) = roster_lost.get("agents") else {
            panic!("the readable part of a damaged roster remains visible");
        };
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].get_str("reason"), Some("blocked"));
    }

    // ---- SC-522: one clock for the stamp and for the relation it justifies ---

    /// A real session directory whose only event is an `ask` nobody answered.
    ///
    /// Written to disk and read back through [`RecordSnapshot::read`] on purpose:
    /// the proof below is about the CLOCK, so every other input has to travel the
    /// production path rather than be hand-assembled around it.
    struct Unanswered(std::path::PathBuf);

    impl Unanswered {
        fn new(tag: &str, asked_at: Timestamp) -> Self {
            let dir = std::env::temp_dir().join(format!("ae-listing-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch dir");
            std::fs::write(
                dir.join("meta"),
                "mode=local\nagent.main=claude:lead:e795c9e9\nagent.worker.0=codex:hand:pending\n",
            )
            .expect("writing a fixture");
            std::fs::write(
                dir.join("events.jsonl"),
                format!(
                    concat!(
                        r#"{{"ts":"{}","actor":"claude:lead","action":"ask","#,
                        r#""target":"codex:hand","ref":"ae-1","summary":"nobody answered"}}"#,
                        "\n",
                    ),
                    asked_at,
                ),
            )
            .expect("writing a fixture");
            Self(dir)
        }

        fn snapshot(&self) -> Snapshot {
            Snapshot {
                sessions: vec![Classified {
                    candidate: Candidate {
                        name: "waiting".to_owned(),
                        durable: Some(DurableRecord {
                            path: self.0.clone(),
                            name: "waiting".to_owned(),
                            layout: Layout::Canonical,
                            server: ServerSelector::Missing,
                            meta_read: MetaRead::Parsed,
                            snapshot: RecordSnapshot::read(&self.0),
                        }),
                        live: None,
                    },
                    status: Status::Running,
                }],
                incomplete: Vec::new(),
            }
        }
    }

    impl Drop for Unanswered {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The stamp and the three SC-017g members, read out of ONE rendered
    /// document, EXACTLY as the document holds them.
    ///
    /// Returns the raw members rather than normalised values. An absent member,
    /// a null and a zero are three different documents, and a helper that folded
    /// them together would let the threshold arm below pass for the wrong reason
    /// — which is what an earlier version of it did.
    fn attention_beside_its_stamp(
        document: &json::Value,
    ) -> (
        Timestamp,
        Option<&json::Value>,
        Option<&json::Value>,
        Option<&json::Value>,
    ) {
        let stamped = Timestamp::parse(
            document
                .get_str("generated_at")
                .expect("SC-509 stamps every document"),
        )
        .expect("the stamp is the documented spelling");
        let Some(json::Value::Arr(sessions)) = document.get("sessions") else {
            panic!("SC-509 renders sessions as an array")
        };
        let session = sessions.first().expect("the one selected session");
        (
            stamped,
            session.get("needs_attention"),
            session.get("attention"),
            session.get("attention_rank"),
        )
    }

    #[test]
    fn sc_522_the_stamp_and_the_attention_it_justifies_cannot_describe_two_moments() {
        // **THE PRECONDITION, not a convenience.** `unanswered` is RELATIONAL on
        // the reader's clock — SC-522 makes it true only once the age EXCEEDS the
        // threshold — while `generated_at` is that same clock printed. Sample
        // twice and the document can assert a stamp that contradicts the
        // attention beside it: a reader cannot tell which moment the digest
        // describes, and the two halves were never true together.
        //
        // The expectation is DERIVED FROM THE DOCUMENT'S OWN STAMP rather than
        // from the local variable. Asserting both against `now` independently
        // would pass on two coincidences; recomputing the relation from the stamp
        // that was actually printed is what closes the hole.
        //
        // **WHAT THIS DOES AND DOES NOT ESTABLISH**, scoped rather than swept
        // (pubfp's sharpening, and it is the accurate statement): the guarantee
        // is OBSERVATIONAL. What is foreclosed is a second clock whose value
        // DIFFERS — used for the entries it disagrees with the parsed
        // `generated_at` at an exact-threshold arm, used for the stamp it
        // disagrees with the supplied `World.now`. Two samplings landing in the
        // same second pass this test, and correctly so: they are
        // indistinguishable in the document and therefore not a contradiction in
        // it. So this pins SELF-CONSISTENCY, not the call count. The call count
        // is a separate, and weaker, fact about `current_world`.
        let asked_at = Timestamp::from_epoch(1_780_000_000);
        let fixture = Unanswered::new("sc522", asked_at);
        let snapshot = fixture.snapshot();

        // Both arms read the SAME bytes. Only the supplied clock moves, so any
        // difference in the answer is the clock's doing and nothing else's.
        //
        // `--all` rather than `--needs-attn` is load-bearing: this proof compares
        // the session's FIELDS across the boundary, so the session has to be
        // present in both arms. Under `--needs-attn` the threshold arm selects
        // nothing and there would be no fields left to disagree about.
        for (label, offset) in [
            ("at the threshold", DEFAULT_UNANSWERED_SECS),
            ("one second past it", DEFAULT_UNANSWERED_SECS + 1),
        ] {
            let now = Timestamp::from_epoch(asked_at.epoch() + offset);
            let world = Presentation::enter(&snapshot).world(now, DEFAULT_UNANSWERED_SECS);
            let rendered = render(&args(&["--all", "--json"]), &world);
            let document = json::parse(&rendered).expect("one complete document");
            let (stamped, needs, attention, rank) = attention_beside_its_stamp(&document);

            // 1. The stamp IS the clock the caller supplied. A `generated_at`
            //    re-sampled inside render would be the wall clock and fail here.
            assert_eq!(
                stamped, now,
                "{label}: the document stamps the supplied now"
            );

            // 2. The relation the document reports is the one ITS OWN stamp
            //    implies. A second sampling behind the fields would compute from
            //    a different moment and disagree with the stamp it printed.
            let implied = asked_at.seconds_until(stamped) > DEFAULT_UNANSWERED_SECS;
            assert_eq!(
                implied,
                offset > DEFAULT_UNANSWERED_SECS,
                "{label}: the fixture sits where this test says it sits"
            );
            assert_eq!(
                needs,
                Some(&json::Value::Bool(implied)),
                "{label}: needs_attention follows the stamp"
            );

            // FLIPPED by colead's ruling (2026-08-24). This arm previously
            // asserted the two optional members were ABSENT below the threshold,
            // and that enforced the wrong letter: absence is SC-509b's spelling
            // for a fact that could not be READ, so a quiet entry that omits them
            // makes loss and legitimate-none the same bytes. Both members are now
            // PRESENT on both sides of the boundary, and only their VALUES move —
            // which is a stronger statement of the same relation, because the
            // member set no longer changes with the clock.
            assert_eq!(
                attention,
                Some(&if implied {
                    json::Value::Str("unanswered".to_owned())
                } else {
                    json::Value::Null
                }),
                "{label}: attention is present either way and follows the stamp"
            );
            assert_eq!(
                rank,
                Some(&json::Value::Num(if implied {
                    Reason::Unanswered.rank()
                } else {
                    0
                })),
                "{label}: attention_rank is present either way and follows the stamp"
            );
        }
    }
}
