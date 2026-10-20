//! Why a session's seat wants attention, which reason wins, and which of them
//! need the human.
//!
//! The attention marker is "the single most-actionable reason by
//! documented severity: dead > stale > waiting-user > blocked > limit >
//! throttled, derived as a rollup across the session's agents".
//! `limit` and `blocked` share rank 3, broken toward the broader word. The digest
//! carries the same fact twice: `attention` (the name) and
//! `attention_rank` (the number, `dead` 6 → `throttled` 2). Rank 1 is retired:
//! an unanswered request is request debt, counted beside the marker
//! (`unanswered`), never a reason.
//!
//! Severity is therefore not a comparison written at each call site — it is the
//! type's own [`Ord`], defined once from the rank the contract publishes.
//! Whether a reason needs the HUMAN is a second, separate fact,
//! [`Reason::needs_you`]: `stale` is the one reason that does not.

use std::fmt;

/// A reason a session needs attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// Rank 2 — an agent is being rate-limited upstream.
    Throttled,
    /// Rank 3 — the vendor's own usage limit reached the pane; the human
    /// waits. Declared before `Blocked` so `Ord` ties toward the broader word.
    Limit,
    /// Rank 3 — an agent is blocked: it declared a concrete external
    /// dependency, or a `waiting-agent` declaration outlived its ceiling.
    Blocked,
    /// Rank 4 — an agent declared it is waiting on the human.
    WaitingUser,
    /// Rank 5 — the watchdog gave up nudging an idle agent (max nudges).
    Stale,
    /// Rank 6 — an agent's pane vanished, or the watchdog flagged it missing.
    Dead,
}

impl Reason {
    /// Every reason, most severe first — the order, written once.
    pub const BY_SEVERITY: [Self; 6] = [
        Self::Dead,
        Self::Stale,
        Self::WaitingUser,
        Self::Blocked,
        Self::Limit,
        Self::Throttled,
    ];

    /// The numeric severity published as `attention_rank`; `Limit` ties `Blocked`.
    ///
    /// ```
    /// use ae::attention::Reason;
    /// assert_eq!(Reason::Dead.rank(), 6);
    /// assert_eq!(Reason::Blocked.rank(), 3);
    /// assert_eq!(Reason::Limit.rank(), Reason::Blocked.rank());
    /// assert_eq!(Reason::Throttled.rank(), 2);
    /// ```
    #[must_use]
    pub const fn rank(self) -> i64 {
        match self {
            Self::Throttled => 2,
            Self::Blocked | Self::Limit => 3,
            Self::WaitingUser => 4,
            Self::Stale => 5,
            Self::Dead => 6,
        }
    }

    /// Whether this is the maximum class in the severity set.
    #[must_use]
    pub fn is_severity_maximum(self) -> bool {
        self == Self::BY_SEVERITY[0]
    }

    /// The spelling published as `attention`, and shown by `ae list` after
    /// `attn:`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Throttled => "throttled",
            Self::Blocked => "blocked",
            Self::Limit => "limit",
            Self::WaitingUser => "waiting-user",
            Self::Stale => "stale",
            Self::Dead => "dead",
        }
    }

    /// The session-level marker: the single most-actionable reason across the
    /// session's agents, or `None` when nothing needs attention.
    ///
    /// ```
    /// use ae::attention::Reason;
    /// // A stale agent and a blocked one: stale is the more actionable.
    /// let rolled = Reason::rollup([Reason::Blocked, Reason::Stale]);
    /// assert_eq!(rolled, Some(Reason::Stale));
    /// assert_eq!(Reason::rollup([]), None);
    /// ```
    #[must_use]
    pub fn rollup<I: IntoIterator<Item = Self>>(reasons: I) -> Option<Self> {
        reasons.into_iter().max()
    }

    /// Whether this reason needs the HUMAN: every reason but `stale`, which is
    /// the watchdog giving up on a quiet seat — shown, never claimed as the
    /// human's. The ONE table every needs-you surface reads.
    ///
    /// ```
    /// use ae::attention::Reason;
    /// assert!(Reason::WaitingUser.needs_you());
    /// assert!(Reason::Throttled.needs_you());
    /// assert!(!Reason::Stale.needs_you());
    /// ```
    #[must_use]
    pub const fn needs_you(self) -> bool {
        match self {
            Self::Dead | Self::WaitingUser | Self::Blocked | Self::Limit | Self::Throttled => true,
            Self::Stale => false,
        }
    }

    /// The most actionable reason among those that need the human, or `None`.
    ///
    /// ```
    /// use ae::attention::Reason;
    /// // A stale seat beside a blocked one: the human is needed for the block.
    /// let rolled = Reason::needs_you_rollup([Reason::Stale, Reason::Blocked]);
    /// assert_eq!(rolled, Some(Reason::Blocked));
    /// assert_eq!(Reason::needs_you_rollup([Reason::Stale]), None);
    /// ```
    #[must_use]
    pub fn needs_you_rollup<I: IntoIterator<Item = Self>>(reasons: I) -> Option<Self> {
        Self::rollup(reasons.into_iter().filter(|reason| reason.needs_you()))
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Reason;

    #[test]
    fn sc_509_ranks_keep_six_through_two_and_retire_unanswered_one() {
        // Existing severity meanings stay fixed; rank 1 (unanswered) is retired.
        // Blocked and limit keep the published rank-3 tie.
        assert_eq!(
            Reason::BY_SEVERITY.map(Reason::rank),
            [6, 5, 4, 3, 3, 2],
            "the published ranks, with limit exactly blocked's"
        );
        assert_eq!(Reason::Blocked.rank(), 3);
        assert_eq!(Reason::Limit.rank(), Reason::Blocked.rank());
    }

    #[test]
    fn sc_017g_severity_orders_dead_over_stale_over_waiting_over_blocked_over_throttled() {
        // The row's order, asserted as the pairwise chain it claims. Ranks
        // are non-increasing because `limit` is the ONE documented tie.
        for pair in Reason::BY_SEVERITY.windows(2) {
            let (more, less) = (pair[0], pair[1]);
            assert!(more > less, "{more} should outrank {less}");
            assert!(
                more.rank() >= less.rank(),
                "{more} should not weigh less than {less}"
            );
        }
    }

    #[test]
    fn sc_017g_the_rollup_is_the_single_most_actionable_reason() {
        assert_eq!(
            Reason::rollup([Reason::Throttled, Reason::Dead, Reason::Blocked]),
            Some(Reason::Dead)
        );
        assert_eq!(
            Reason::rollup([Reason::Throttled, Reason::Limit]),
            Some(Reason::Limit)
        );
        // The documented rank-3 tie breaks toward the broader word.
        assert_eq!(
            Reason::rollup([Reason::Blocked, Reason::Limit]),
            Some(Reason::Blocked)
        );
        // Order of arrival must not decide it.
        assert_eq!(
            Reason::rollup([Reason::Dead, Reason::Throttled]),
            Reason::rollup([Reason::Throttled, Reason::Dead])
        );
    }

    #[test]
    fn a_session_with_no_reason_has_no_marker() {
        assert_eq!(Reason::rollup([]), None);
    }

    #[test]
    fn every_reason_publishes_the_spelling_the_digest_carries() {
        assert_eq!(
            Reason::BY_SEVERITY.map(Reason::as_str),
            [
                "dead",
                "stale",
                "waiting-user",
                "blocked",
                "limit",
                "throttled"
            ]
        );
    }

    #[test]
    fn only_stale_is_a_reason_the_human_is_not_needed_for() {
        // The human ruling: session status means whether the human is needed.
        let quiet: Vec<_> = Reason::BY_SEVERITY
            .into_iter()
            .filter(|reason| !reason.needs_you())
            .collect();
        assert_eq!(quiet, [Reason::Stale]);
        assert_eq!(
            Reason::needs_you_rollup([Reason::Stale, Reason::Throttled]),
            Some(Reason::Throttled),
            "stale's severity cannot hide a reason that needs the human"
        );
        assert_eq!(Reason::needs_you_rollup([Reason::Stale]), None);
    }

    #[test]
    fn a_reason_displays_as_the_spelling_it_publishes() {
        // `ae list` shows this after `attn:`, so Display and as_str are one
        // fact, not two that happen to agree today.
        for reason in Reason::BY_SEVERITY {
            assert_eq!(format!("{reason}"), reason.as_str());
            assert_eq!(
                format!("attn:{reason}"),
                format!("attn:{}", reason.as_str())
            );
        }
    }

    #[test]
    fn the_severity_list_holds_every_variant_exactly_once() {
        // A new reason added to the enum but not to BY_SEVERITY would silently
        // fall out of the rollup, the ranks and the round-trip above.
        let mut seen = Reason::BY_SEVERITY.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), Reason::BY_SEVERITY.len());
    }

    #[test]
    fn only_the_severity_maximum_is_marked_from_the_complete_set() {
        assert_eq!(Reason::BY_SEVERITY[0], Reason::Dead);
        assert!(Reason::BY_SEVERITY[0].is_severity_maximum());
        assert!(
            Reason::BY_SEVERITY[1..]
                .iter()
                .all(|reason| !reason.is_severity_maximum()),
            "the maximum marker tracks the complete severity set"
        );
    }
}
