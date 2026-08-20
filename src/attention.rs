//! Why a session wants a human, and which reason wins.
//!
//! **SC-017g** — the attention marker is "the single most-actionable reason by
//! documented severity: dead > stale > waiting-user > blocked > throttled >
//! unanswered, derived as a rollup across the session's agents". **SC-509**
//! carries the same fact twice in the digest: `attention` (the name) and
//! `attention_rank` (the number, `dead` 6 → `unanswered` 1).
//!
//! Severity is therefore not a comparison written at each call site — it is the
//! type's own [`Ord`], defined once from the rank the contract publishes.

use std::fmt;

/// A reason a session needs attention, per SC-017g.
///
/// Ordered by severity, so `max()` *is* the rollup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// Rank 1 — an inter-agent `ask`/`review` went unanswered past the
    /// threshold (`AE_ATTN_REQUEST_SECS`, default 30 min). The lowest severity.
    Unanswered,
    /// Rank 2 — an agent is being rate-limited upstream.
    Throttled,
    /// Rank 3 — an agent declared it is blocked on an external dependency.
    Blocked,
    /// Rank 4 — an agent declared it is waiting on the human.
    WaitingUser,
    /// Rank 5 — the watchdog gave up nudging an idle agent (max nudges).
    Stale,
    /// Rank 6 — an agent's pane vanished, or the watchdog flagged it missing.
    Dead,
}

impl Reason {
    /// Every reason, most severe first — the SC-017g order, written once.
    pub const BY_SEVERITY: [Self; 6] = [
        Self::Dead,
        Self::Stale,
        Self::WaitingUser,
        Self::Blocked,
        Self::Throttled,
        Self::Unanswered,
    ];

    /// The numeric severity SC-509 publishes as `attention_rank`.
    ///
    /// ```
    /// use ae::attention::Reason;
    /// assert_eq!(Reason::Dead.rank(), 6);
    /// assert_eq!(Reason::Blocked.rank(), 3);
    /// assert_eq!(Reason::Unanswered.rank(), 1);
    /// ```
    #[must_use]
    pub const fn rank(self) -> i64 {
        match self {
            Self::Unanswered => 1,
            Self::Throttled => 2,
            Self::Blocked => 3,
            Self::WaitingUser => 4,
            Self::Stale => 5,
            Self::Dead => 6,
        }
    }

    /// The spelling SC-509 publishes as `attention`, and `ae list` shows after
    /// `attn:`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unanswered => "unanswered",
            Self::Throttled => "throttled",
            Self::Blocked => "blocked",
            Self::WaitingUser => "waiting-user",
            Self::Stale => "stale",
            Self::Dead => "dead",
        }
    }

    /// The inverse of [`Reason::as_str`]. Unknown text is not a reason.
    #[must_use]
    pub fn from_str_exact(text: &str) -> Option<Self> {
        Self::BY_SEVERITY
            .into_iter()
            .find(|reason| reason.as_str() == text)
    }

    /// The session-level marker: the single most-actionable reason across the
    /// session's agents (SC-017g), or `None` when nothing needs attention.
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
    fn sc_509_ranks_run_from_dead_six_to_unanswered_one() {
        // commands.md: "attention_rank is the numeric severity (dead 6 →
        // unanswered 1)", and its worked example pairs "blocked" with 3.
        assert_eq!(
            Reason::BY_SEVERITY.map(Reason::rank),
            [6, 5, 4, 3, 2, 1],
            "the published ranks"
        );
        assert_eq!(Reason::Blocked.rank(), 3);
    }

    #[test]
    fn sc_017g_severity_orders_dead_over_stale_over_waiting_over_blocked_over_throttled_over_unanswered()
     {
        // The row's order, asserted as the pairwise chain it claims.
        for pair in Reason::BY_SEVERITY.windows(2) {
            let (more, less) = (pair[0], pair[1]);
            assert!(more > less, "{more} should outrank {less}");
            assert!(more.rank() > less.rank(), "{more} should outrank {less}");
        }
    }

    #[test]
    fn sc_017g_the_rollup_is_the_single_most_actionable_reason() {
        assert_eq!(
            Reason::rollup([Reason::Unanswered, Reason::Dead, Reason::Blocked]),
            Some(Reason::Dead)
        );
        assert_eq!(
            Reason::rollup([Reason::Throttled, Reason::Unanswered]),
            Some(Reason::Throttled)
        );
        // Order of arrival must not decide it.
        assert_eq!(
            Reason::rollup([Reason::Dead, Reason::Unanswered]),
            Reason::rollup([Reason::Unanswered, Reason::Dead])
        );
    }

    #[test]
    fn a_session_with_no_reason_has_no_marker() {
        assert_eq!(Reason::rollup([]), None);
    }

    #[test]
    fn every_reason_round_trips_through_its_published_spelling() {
        for reason in Reason::BY_SEVERITY {
            assert_eq!(Reason::from_str_exact(reason.as_str()), Some(reason));
        }
        assert_eq!(
            Reason::BY_SEVERITY.map(Reason::as_str),
            [
                "dead",
                "stale",
                "waiting-user",
                "blocked",
                "throttled",
                "unanswered"
            ]
        );
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
    fn text_that_is_not_a_documented_reason_is_not_one() {
        for other in ["", "DEAD", "waiting_user", "working", "done", "idle"] {
            assert_eq!(Reason::from_str_exact(other), None, "{other:?}");
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
}
