//! Which sessions `ae list` shows, as pure functions.
//!
//! | Flag | Shows |
//! |---|---|
//! | *(none)* | running sessions only — stopped history is opt-in noise |
//! | `--running` | the explicit spelling of that default |
//! | `--all` | running sessions, **then** stopped |
//! | `--stopped` | stopped sessions only |
//! | `--needs-attn` | attention sessions; `--needs-me`/`--needs`/`--attn` |
//! | `--active` | recent activity — an ae event within ~5 min, `AE_LIST_ACTIVE_SECS` tunes, `--busy` alias |
//! | `--json` | honours the active filters |
//!
//! `--json` is why this module exists at all: the digest and the table must not
//! each carry their own idea of what "the active filters" selected. Selection
//! happens once, over the model, and both renderings consume the result.
//!
//! Flags COMBINE in two ways, because they are of two different kinds:
//! flags are of two different kinds:
//!
//! * **same dimension** — `--running` / `--stopped` / `--all` are
//!   dimension and therefore ALTERNATIVES: the last distinct selector wins, and
//!   repeating one changes nothing.
//! * **across dimensions** — the filters INTERSECT
//!   literally, so `--stopped --needs-attn` selects nothing rather than
//!   erroring, because each attention/activity row reads "running sessions
//!   only" on its own terms.
//!
//! Nothing here reads a clock or a directory. `now` is a parameter, so
//! "recent" is decidable in a test without waiting for time to pass.

use crate::digest::{SessionEntry, Status};
use crate::time::Timestamp;

/// The `--active` window when nothing tunes it.
pub const DEFAULT_ACTIVE_WINDOW_SECS: i64 = 300;

/// Which half of the world a listing covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// running sessions only. The default.
    #[default]
    Running,
    /// running sessions, then stopped ones.
    All,
    /// stopped sessions only.
    Stopped,
}

impl Scope {
    /// The status groups this scope shows, in the order they are shown in.
    const fn order(self) -> &'static [Status] {
        match self {
            Self::Running => &[Status::Running, Status::Unknown],
            Self::Stopped => &[Status::Stopped],
            Self::All => &[Status::Running, Status::Unknown, Status::Stopped],
        }
    }
}

/// The active filters, as one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    /// Which sessions are in scope at all.
    pub scope: Scope,
    /// keep only sessions carrying an attention reason.
    pub needs_attention: bool,
    /// keep only sessions with an ae event within this many seconds.
    /// `None` does not filter on activity.
    pub active_within_secs: Option<i64>,
}

impl Selection {
    /// The default listing: running sessions only.
    #[must_use]
    pub fn running() -> Self {
        Self::default()
    }

    /// Apply the active filters to `sessions`, as of `now`.
    ///
    /// Returns borrowed entries in the order they should be shown: running
    /// sessions first, then unknown ones, then stopped ones,
    /// restricted to the groups the scope carries, each group keeping
    /// the order it arrived in.
    ///
    /// ```
    /// use ae::digest::{SessionEntry, Status};
    /// use ae::filters::Selection;
    /// use ae::time::Timestamp;
    ///
    /// let sessions = vec![
    ///     SessionEntry::new("old", Status::Stopped),
    ///     SessionEntry::new("live", Status::Running),
    /// ];
    /// let shown = Selection::running().select(&sessions, Timestamp::from_epoch(0));
    /// assert_eq!(shown.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["live"]);
    /// ```
    #[must_use]
    pub fn select<'a>(
        &self,
        sessions: &'a [SessionEntry],
        now: Timestamp,
    ) -> Vec<&'a SessionEntry> {
        // The scope names a composition AND an order, so it carries both
        // rather than leaving two call sites to agree by
        // coincidence. See [`Scope::order`].
        self.scope
            .order()
            .iter()
            .flat_map(|status| {
                let mut group: Vec<&SessionEntry> = sessions
                    .iter()
                    .filter(|session| session.status == *status)
                    .filter(|session| self.keeps(session, now))
                    .collect();
                // the PRODUCT sorts. Raw byte / `LC_ALL=C` order
                // by name within the group, so tmux emission order, filesystem
                // glob order, root traversal, locale collation and creation
                group.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
                group
            })
            .collect()
    }

    /// Whether the attention and activity filters keep `session`.
    fn keeps(&self, session: &SessionEntry, now: Timestamp) -> bool {
        if self.needs_attention && !(live_scope(session.status) && session.needs_attention()) {
            return false;
        }
        if let Some(window) = self.active_within_secs
            && !(live_scope(session.status) && is_active(session, now, window))
        {
            return false;
        }
        true
    }
}

/// Whether a status is in live scope — every session not known
/// stopped.
const fn live_scope(status: Status) -> bool {
    match status {
        Status::Running | Status::Unknown => true,
        Status::Stopped => false,
    }
}

/// Whether `session` saw an ae event within `window` seconds of `now`.
fn is_active(session: &SessionEntry, now: Timestamp, window: i64) -> bool {
    session.last_active_epoch.is_some_and(|epoch| {
        let age = Timestamp::from_epoch(epoch).seconds_until(now);
        age <= window
    })
}

/// The `list` argv, once read: which sessions, and in which rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListArgs {
    /// The filters the flags selected.
    pub selection: Selection,
    /// whether to render the machine-readable digest.
    pub json: bool,
}

/// A flag `list` does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFlag(pub String);

impl std::fmt::Display for UnknownFlag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown argument: {}", self.0)
    }
}

impl std::error::Error for UnknownFlag {}

impl ListArgs {
    /// Read `list`'s flags — argv AFTER the `list` / `ls` word.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownFlag`] for an argument no row names. What the binary
    /// then does with it is a dispatch decision, not this function's.
    ///
    /// ```
    /// use ae::filters::{ListArgs, Scope};
    ///
    /// let args = ListArgs::parse(&["--all".to_owned(), "--json".to_owned()])?;
    /// assert_eq!(args.selection.scope, Scope::All);
    /// assert!(args.json);
    /// # Ok::<(), ae::filters::UnknownFlag>(())
    /// ```
    pub fn parse<S: AsRef<str>>(args: &[S]) -> Result<Self, UnknownFlag> {
        let mut parsed = Self::default();
        for arg in args {
            match arg.as_ref() {
                // one dimension, so these are
                // ALTERNATIVES: assignment, not accumulation,
                // which makes the last distinct selector win and a repeat a
                "--running" => parsed.selection.scope = Scope::Running,
                "--all" => parsed.selection.scope = Scope::All,
                "--stopped" => parsed.selection.scope = Scope::Stopped,
                // The attention filter, with its three aliases.
                "--needs-attn" | "--needs-me" | "--needs" | "--attn" => {
                    parsed.selection.needs_attention = true;
                }
                // The activity filter, with its `--busy` alias. The window is
                // "~5 min"; a caller that reads AE_LIST_ACTIVE_SECS overwrites
                // it, because this function does not read the environment.
                "--active" | "--busy" => {
                    parsed.selection.active_within_secs = Some(DEFAULT_ACTIVE_WINDOW_SECS);
                }
                // The JSON rendering.
                "--json" => parsed.json = true,
                other => return Err(UnknownFlag(other.to_owned())),
            }
        }
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_ACTIVE_WINDOW_SECS, ListArgs, Scope, Selection, UnknownFlag};
    use crate::attention::Reason;
    use crate::digest::{SessionEntry, Status};
    use crate::time::Timestamp;

    const NOW: Timestamp = Timestamp::from_epoch(1_780_000_000);

    fn session(name: &str, status: Status) -> SessionEntry {
        SessionEntry::new(name, status)
    }

    fn attn(name: &str, status: Status, reason: Reason) -> SessionEntry {
        let mut entry = session(name, status);
        entry.attention = Some(reason);
        entry
    }

    fn active(name: &str, status: Status, seconds_ago: i64) -> SessionEntry {
        let mut entry = session(name, status);
        entry.last_active_epoch = Some(NOW.epoch() - seconds_ago);
        entry
    }

    fn names<'a>(shown: &[&'a SessionEntry]) -> Vec<&'a str> {
        shown.iter().map(|s| s.name.as_str()).collect()
    }

    fn corpus() -> Vec<SessionEntry> {
        vec![
            session("stopped-first", Status::Stopped),
            session("running-one", Status::Running),
            session("unknown-first", Status::Unknown),
            session("stopped-second", Status::Stopped),
            session("running-two", Status::Running),
            session("unknown-second", Status::Unknown),
        ]
    }

    #[test]
    fn sc_017m_each_scope_shows_exactly_this_composition_in_this_order() {
        // Transcription of the ratified order, and not
        // off the implementation.
        assert_eq!(
            Scope::Running.order(),
            [Status::Running, Status::Unknown],
            "default / --running"
        );
        assert_eq!(Scope::Stopped.order(), [Status::Stopped], "--stopped");
        assert_eq!(
            Scope::All.order(),
            [Status::Running, Status::Unknown, Status::Stopped],
            "--all"
        );
        assert_eq!(
            Scope::default().order(),
            Scope::Running.order(),
            "the default IS --running, so it cannot show a different set"
        );
    }

    #[test]
    fn sc_017m_no_status_falls_out_of_every_scope() {
        // THE anti-silent-hole mechanism. Rust checks `match` for
        // exhaustiveness and nothing else, so the arrays in `Scope::order` will
        // compile forever after a variant is added to `Status` — dropping that
        for status in Status::ALL {
            assert!(
                [Scope::Running, Scope::All, Scope::Stopped]
                    .iter()
                    .any(|scope| scope.order().contains(&status)),
                "no scope shows {status:?} — it is invisible to every `ae list`"
            );
        }
    }

    #[test]
    fn sc_017m_unknown_is_visible_by_default_because_it_is_not_stopped_history() {
        // The point of the ruling: a session whose liveness ae could not
        // establish must not be hidden by the listing a human runs bare.
        let corpus = corpus();
        let shown = Selection::default().select(&corpus, NOW);
        assert_eq!(
            names(&shown),
            [
                "running-one",
                "running-two",
                "unknown-first",
                "unknown-second"
            ]
        );
        assert!(
            !Scope::Stopped.order().contains(&Status::Unknown),
            "and --stopped is history, which unknown is not"
        );
    }

    #[test]
    fn sc_017a_bare_list_selects_the_default() {
        let args = ListArgs::parse::<String>(&[]).expect("no flags is legal");
        assert_eq!(args, ListArgs::default());
        assert_eq!(args.selection.scope, Scope::Running);
        assert!(!args.json);
    }

    #[test]
    fn every_documented_scope_flag_has_its_row_s_effect() {
        for (flag, scope) in [
            ("--running", Scope::Running),
            ("--all", Scope::All),
            ("--stopped", Scope::Stopped),
        ] {
            let args = ListArgs::parse(&[flag]).expect("a documented flag");
            assert_eq!(args.selection.scope, scope, "{flag}");
        }
    }

    #[test]
    fn sc_017d_every_documented_alias_reaches_the_same_filter() {
        // "aliases: --needs-me, --needs, --attn".
        for flag in ["--needs-attn", "--needs-me", "--needs", "--attn"] {
            let args = ListArgs::parse(&[flag]).expect("a documented alias");
            assert!(args.selection.needs_attention, "{flag}");
        }
    }

    #[test]
    fn sc_017e_active_and_its_alias_set_the_documented_window() {
        for flag in ["--active", "--busy"] {
            let args = ListArgs::parse(&[flag]).expect("a documented alias");
            assert_eq!(
                args.selection.active_within_secs,
                Some(DEFAULT_ACTIVE_WINDOW_SECS),
                "{flag}"
            );
        }
    }

    #[test]
    fn sc_017f_json_is_a_rendering_not_a_filter() {
        // The row: --json HONOURS the active filters. It must not change them.
        let with = ListArgs::parse(&["--all", "--json"]).expect("legal");
        let without = ListArgs::parse(&["--all"]).expect("legal");
        assert_eq!(with.selection, without.selection);
        assert!(with.json);
        assert!(!without.json);
    }

    #[test]
    fn flags_accumulate_rather_than_replacing_each_other() {
        let args = ListArgs::parse(&["--all", "--needs-attn", "--busy", "--json"]).expect("legal");
        assert_eq!(args.selection.scope, Scope::All);
        assert!(args.selection.needs_attention);
        assert_eq!(
            args.selection.active_within_secs,
            Some(DEFAULT_ACTIVE_WINDOW_SECS)
        );
        assert!(args.json);
    }

    #[test]
    fn sc_521_amended_same_dimension_scope_flags_are_alternatives() {
        // The seat ruling: --running/--stopped/--all select the same dimension,
        // so the LAST DISTINCT selector wins rather than the combination being
        // an error or an intersection.
        assert_eq!(
            ListArgs::parse(&["--all", "--stopped"])
                .expect("legal")
                .selection
                .scope,
            Scope::Stopped
        );
        assert_eq!(
            ListArgs::parse(&["--stopped", "--all"])
                .expect("legal")
                .selection
                .scope,
            Scope::All,
            "order decides, so the reverse pair gives the reverse answer"
        );
        assert_eq!(
            ListArgs::parse(&["--stopped", "--all", "--running"])
                .expect("legal")
                .selection
                .scope,
            Scope::Running,
            "and the last of three still wins"
        );
    }

    #[test]
    fn sc_521_amended_a_repeated_scope_flag_is_idempotent() {
        for flag in ["--running", "--all", "--stopped"] {
            let once = ListArgs::parse(&[flag]).expect("legal");
            let thrice = ListArgs::parse(&[flag, flag, flag]).expect("legal");
            assert_eq!(once, thrice, "{flag} repeated changes nothing");
        }
        // And a repeat after a different selector does not resurrect the older
        // one: the last DISTINCT selector is still the winner.
        assert_eq!(
            ListArgs::parse(&["--all", "--stopped", "--stopped"])
                .expect("legal")
                .selection
                .scope,
            Scope::Stopped
        );
    }

    #[test]
    fn a_flag_no_row_names_is_refused_with_the_argument_verbatim() {
        let err = ListArgs::parse(&["--frobnicate"]).expect_err("not a documented flag");
        assert_eq!(err, UnknownFlag("--frobnicate".to_owned()));
        assert!(err.to_string().contains("--frobnicate"));
    }

    #[test]
    fn sc_017a_amended_the_default_shows_no_stopped_session() {
        // The default is no longer
        // "running only" but "not stopped history" — running plus unknown. What
        // still holds, and what this asserts, is that a stopped session
        let corpus = corpus();
        let shown = Selection::default().select(&corpus, NOW);
        assert!(
            shown.iter().all(|s| s.status != Status::Stopped),
            "{:?}",
            names(&shown)
        );
        assert_eq!(
            names(&shown),
            [
                "running-one",
                "running-two",
                "unknown-first",
                "unknown-second"
            ]
        );
    }

    #[test]
    fn sc_017i_running_is_the_explicit_spelling_of_the_default() {
        let corpus = corpus();
        assert_eq!(
            Selection::running().select(&corpus, NOW),
            Selection::default().select(&corpus, NOW)
        );
        assert_eq!(Selection::running().scope, Scope::Running);
    }

    #[test]
    fn sc_017b_all_shows_running_sessions_then_unknown_ones_then_stopped_ones() {
        // The row names an ORDER, not just a wider set: the stopped session
        // that comes first in the corpus must still come last in the listing,
        // and unknown sits between the two groups.
        let selection = Selection {
            scope: Scope::All,
            ..Selection::default()
        };
        let corpus = corpus();
        let shown = selection.select(&corpus, NOW);
        assert_eq!(
            names(&shown),
            [
                "running-one",
                "running-two",
                "unknown-first",
                "unknown-second",
                "stopped-first",
                "stopped-second"
            ]
        );
    }

    #[test]
    fn sc_017c_stopped_shows_stopped_sessions_only() {
        let selection = Selection {
            scope: Scope::Stopped,
            ..Selection::default()
        };
        let corpus = corpus();
        let shown = selection.select(&corpus, NOW);
        assert_eq!(names(&shown), ["stopped-first", "stopped-second"]);
    }

    #[test]
    fn sc_017d_needs_attn_keeps_only_sessions_with_a_reason() {
        let sessions = vec![
            session("quiet", Status::Running),
            attn("blocked-one", Status::Running, Reason::Blocked),
            attn("dead-one", Status::Running, Reason::Dead),
        ];
        let selection = Selection {
            needs_attention: true,
            ..Selection::default()
        };
        let shown = selection.select(&sessions, NOW);
        assert_eq!(names(&shown), ["blocked-one", "dead-one"]);
    }

    #[test]
    fn sc_017d_needs_attn_is_about_running_sessions() {
        // "only RUNNING sessions with an attn: reason".
        let sessions = vec![attn("stopped-but-flagged", Status::Stopped, Reason::Dead)];
        let selection = Selection {
            needs_attention: true,
            ..Selection::default()
        };
        assert!(selection.select(&sessions, NOW).is_empty());

        let widened = Selection {
            scope: Scope::All,
            needs_attention: true,
            ..Selection::default()
        };
        assert!(
            widened.select(&sessions, NOW).is_empty(),
            "widening the scope does not make a stopped session an attention session"
        );
    }

    #[test]
    fn sc_017e_active_keeps_sessions_with_an_event_inside_the_window() {
        let sessions = vec![
            active("just-now", Status::Running, 1),
            active("four-minutes-ago", Status::Running, 240),
            active("ten-minutes-ago", Status::Running, 600),
            session("never", Status::Running),
        ];
        let selection = Selection {
            active_within_secs: Some(DEFAULT_ACTIVE_WINDOW_SECS),
            ..Selection::default()
        };
        let shown = selection.select(&sessions, NOW);
        // MEMBERSHIP is what this row is about. The sequence is C-byte
        // order, which is why it no longer matches the order they were supplied
        // in — a test that pinned supplied order would now be asserting the very
        assert_eq!(names(&shown), ["four-minutes-ago", "just-now"]);
    }

    #[test]
    fn sc_017e_the_window_is_a_parameter_so_the_env_can_tune_it() {
        // AE_LIST_ACTIVE_SECS tunes it; the exact default is set elsewhere.
        let sessions = vec![active("ten-minutes-ago", Status::Running, 600)];
        let wide = Selection {
            active_within_secs: Some(3600),
            ..Selection::default()
        };
        assert_eq!(names(&wide.select(&sessions, NOW)), ["ten-minutes-ago"]);
    }

    #[test]
    fn sc_017e_the_boundary_second_is_inside_the_window() {
        let sessions = vec![
            active("exactly-at-the-edge", Status::Running, 300),
            active("one-second-past", Status::Running, 301),
        ];
        let selection = Selection {
            active_within_secs: Some(DEFAULT_ACTIVE_WINDOW_SECS),
            ..Selection::default()
        };
        assert_eq!(
            names(&selection.select(&sessions, NOW)),
            ["exactly-at-the-edge"]
        );
    }

    #[test]
    fn sc_524_a_timestamp_from_the_future_counts_as_active() {
        // Loud-direction doctrine: skew shows a session active rather than
        // silently hiding a live one. Both a small skew and an absurd one.
        for skew in [-5, -86_400, -31_536_000] {
            let sessions = vec![active("clock-ahead", Status::Running, skew)];
            let selection = Selection {
                active_within_secs: Some(DEFAULT_ACTIVE_WINDOW_SECS),
                ..Selection::default()
            };
            assert_eq!(
                names(&selection.select(&sessions, NOW)),
                ["clock-ahead"],
                "{skew}"
            );
        }

        let sessions = vec![active("clock-ahead", Status::Running, -5)];
        let selection = Selection {
            active_within_secs: Some(DEFAULT_ACTIVE_WINDOW_SECS),
            ..Selection::default()
        };
        assert_eq!(names(&selection.select(&sessions, NOW)), ["clock-ahead"]);
    }

    #[test]
    fn the_filters_compose_rather_than_override_each_other() {
        let mut busy_and_blocked = attn("both", Status::Running, Reason::Blocked);
        busy_and_blocked.last_active_epoch = Some(NOW.epoch() - 10);
        let sessions = vec![
            busy_and_blocked,
            attn("blocked-but-idle", Status::Running, Reason::Blocked),
            active("busy-but-fine", Status::Running, 10),
        ];
        let selection = Selection {
            needs_attention: true,
            active_within_secs: Some(DEFAULT_ACTIVE_WINDOW_SECS),
            ..Selection::default()
        };
        assert_eq!(names(&selection.select(&sessions, NOW)), ["both"]);
    }

    #[test]
    fn sc_017f_the_digest_and_the_table_select_from_one_answer() {
        // The row: --json honours the active filters. Same Selection, same
        // sessions, same result — there is no second selection path to diverge.
        let sessions = corpus();
        let selection = Selection {
            scope: Scope::All,
            ..Selection::default()
        };
        let for_table = selection.select(&sessions, NOW);
        let for_json = selection.select(&sessions, NOW);
        assert_eq!(names(&for_table), names(&for_json));
    }

    #[test]
    fn an_empty_corpus_selects_nothing_under_every_scope() {
        for scope in [Scope::Running, Scope::All, Scope::Stopped] {
            let selection = Selection {
                scope,
                ..Selection::default()
            };
            assert!(selection.select(&[], NOW).is_empty(), "{scope:?}");
        }
    }
}
