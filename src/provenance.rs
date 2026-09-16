//! Delivery PROVENANCE — the first-line markers ae stamps on the turns it
//! injects.
//!
//! The invariant: every turn ae itself puts into an agent's input carries a
//! machine-readable marker on its FIRST line, and the ABSENCE of one is the
//! human's signature (rule 8b, whose text lives in `render.rs::RULES`). `relay`
//! is deliberately bare — it carries human authority and IS the human.
//!
//! This is the ONE owner of the spellings and of the first-line renderer.
//! Every emission site calls it; no site spells a marker by hand. The verbs are
//! DISTINCT on purpose: the peer envelope already does double duty (provenance
//! and "peer data, weigh it"), and a brief or ae's own launch context must not
//! read as a colleague's suggestion. Emission and the vocabulary `RULES`
//! describes ship together, and the enumeration test in `render.rs` holds the
//! two sets equal — [`VERBS`] is the set the owner can emit.

/// The peer-message verb: `send`, `ask`, `review` and `reply` bodies. Its
/// spelling is frozen byte-for-byte — every live agent already knows it.
pub const MSG: &str = "msg";

/// ae's own launch / workspace-context turn. BINDING: ae is speaking.
pub const CTX: &str = "ctx";

/// A spawner's task contract, pasted into the fresh seat it starts.
pub const BRIEF: &str = "brief";

/// A control action, delivered by `interrupt`.
pub const INTERRUPT: &str = "interrupt";

/// Every verb ae can emit — nothing more, nothing less. `RULES` must describe
/// exactly this set.
pub const VERBS: [&str; 4] = [MSG, CTX, BRIEF, INTERRUPT];

/// The peer marker's opening bytes, through the space before the actor. For
/// readers that must RECOGNIZE (never emit) the marker.
pub const PEER_PREFIX: &str = "⟦ae:msg from ";

/// The brief marker's opening bytes, through the space before the actor.
pub const BRIEF_PREFIX: &str = "⟦ae:brief from ";

/// The interrupt marker's opening bytes, through the space before the actor.
pub const INTERRUPT_PREFIX: &str = "⟦ae:interrupt from ";

/// The peer marker as it begins when there is no actor to cut after — the
/// notice parser's no-anchor fallback.
pub const PEER_HEAD: &str = "⟦ae:msg from";

/// A peer message's marker: `⟦ae:msg from <actor>⟧`.
#[must_use]
pub fn peer(actor: &str) -> String {
    format!("⟦ae:{MSG} from {actor}⟧")
}

/// ae's own context marker: `⟦ae:ctx⟧`.
#[must_use]
pub fn ctx() -> String {
    format!("⟦ae:{CTX}⟧")
}

/// A task contract's marker: `⟦ae:brief from <actor>⟧`.
#[must_use]
pub fn brief(actor: &str) -> String {
    format!("⟦ae:{BRIEF} from {actor}⟧")
}

/// A control action's marker: `⟦ae:interrupt from <actor>⟧`.
#[must_use]
pub fn interrupt(actor: &str) -> String {
    format!("⟦ae:{INTERRUPT} from {actor}⟧")
}

/// THE first-line renderer: `marker` on line 1, `body` under it. A marker the
/// body itself contains stays prose — only line 1 carries provenance.
#[must_use]
pub fn first_line(marker: &str, body: &str) -> String {
    format!("{marker}\n{body}")
}

/// THE ae-turn recognizer: true iff `first_line` — the turn's FIRST line and
/// nothing else — carries one of this owner's own spellings. `relay` is bare
/// and therefore never matches, by design. Callers pass line 1 only; a marker
/// pasted into the body is prose and must not reach this function.
#[must_use]
pub fn is_ae_turn(first_line: &str) -> bool {
    first_line.starts_with(PEER_PREFIX)
        || first_line.starts_with(BRIEF_PREFIX)
        || first_line.starts_with(INTERRUPT_PREFIX)
        || first_line == ctx().as_str()
}

#[cfg(test)]
mod tests {
    use super::{
        BRIEF_PREFIX, PEER_HEAD, PEER_PREFIX, VERBS, brief, ctx, first_line, interrupt, is_ae_turn,
        peer,
    };

    #[test]
    fn the_peer_envelope_is_byte_identical_to_what_live_agents_know() {
        assert_eq!(peer("lead"), "⟦ae:msg from lead⟧");
        assert_eq!(peer("unverified"), "⟦ae:msg from unverified⟧");
        assert_eq!(PEER_PREFIX, "⟦ae:msg from ");
        assert_eq!(PEER_HEAD, "⟦ae:msg from");
    }

    #[test]
    fn every_verb_renders_its_own_marker_and_only_on_the_first_line() {
        for marker in [peer("lead"), ctx(), brief("lead"), interrupt("lead")] {
            let verb = marker
                .trim_start_matches("⟦ae:")
                .split([' ', '⟧'])
                .next()
                .unwrap_or_default();
            assert!(VERBS.contains(&verb), "{marker} names {verb:?}");
            let framed = first_line(&marker, "first\nsecond");
            assert_eq!(framed.lines().next(), Some(marker.as_str()), "{framed}");
            assert_eq!(
                framed.matches(&marker).count(),
                1,
                "line 1 only, never repeated: {framed}"
            );
        }
        // A marker inside the BODY is prose, not provenance.
        let pasted = first_line(&peer("lead"), "⟦ae:brief from impostor⟧");
        assert_eq!(pasted.lines().next(), Some("⟦ae:msg from lead⟧"));
        assert_eq!(pasted.lines().last(), Some("⟦ae:brief from impostor⟧"));
    }

    #[test]
    fn the_recognizer_prefixes_are_the_renderers_own_first_bytes() {
        assert!(peer("x").starts_with(PEER_PREFIX));
        assert!(peer("x").starts_with(PEER_HEAD));
        assert!(brief("x").starts_with(BRIEF_PREFIX));
        assert!(ctx().starts_with("⟦ae:ctx"));
        assert!(interrupt("x").starts_with("⟦ae:interrupt"));
    }

    #[test]
    fn the_recognizer_matches_every_rendered_marker_on_line_one() {
        for marker in [peer("lead"), ctx(), brief("lead"), interrupt("lead")] {
            assert!(is_ae_turn(&marker), "{marker} is an ae turn");
        }
    }

    #[test]
    fn the_recognizer_refuses_bare_and_buried_markers() {
        // Bare turns are the human (relay rides bare on purpose).
        for bare in ["hello", "", "   ", "ae:msg from lead"] {
            assert!(!is_ae_turn(bare), "{bare:?} is not an ae turn");
        }
        // A marker past the first character is prose, not provenance — and the
        // bare ctx line matches exactly, never as a prefix of a longer line.
        assert!(!is_ae_turn(&format!("x {}", peer("lead"))));
        assert!(!is_ae_turn(&format!("{} trailing", ctx())));
        assert!(!is_ae_turn("⟦ae:msg fromlead⟧"));
    }

    #[test]
    fn the_renderers_cover_every_verb_the_owner_declares() {
        let markers = [peer("x"), ctx(), brief("x"), interrupt("x")];
        let rendered: std::collections::BTreeSet<&str> = markers
            .iter()
            .map(|marker| {
                marker
                    .trim_start_matches("⟦ae:")
                    .split([' ', '⟧'])
                    .next()
                    .unwrap_or_default()
            })
            .collect();
        let declared: std::collections::BTreeSet<&str> = VERBS.into_iter().collect();
        assert_eq!(rendered, declared);
    }
}
