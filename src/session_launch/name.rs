//! Session names: the grammar every explicit launch name must satisfy.
//!
//! Ported from `ae`'s `_validate_session_name`. The grammar is an ALLOWLIST
//! because a session name becomes a tmux session, a directory under
//! `~/.ae/sessions`, part of `.lifecycle.<name>.lock`, a neighbour in tmux
//! format strings, and the target of the launch rollback's recursive delete.

/// The session-name grammar, echoed verbatim in the refusal.
pub(crate) const SESSION_NAME_GRAMMAR: &str = "^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$";

/// Whether `name` is a legal session name.
pub(crate) fn is_session_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    if name.len() > 128 {
        return false;
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Whether `candidate` is a DIRECT child of `parent`, by pure string structure
/// — the belt to entry validation's braces before anything is deleted.
pub(crate) fn is_direct_child(parent: &str, candidate: &str) -> bool {
    if parent.is_empty() || candidate.is_empty() {
        return false;
    }
    let Some((head, base)) = candidate.rsplit_once('/') else {
        return false;
    };
    head == parent && !base.is_empty() && base != "." && base != ".."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grammar_refuses_what_it_says_it_refuses() {
        assert!(is_session_name("a"));
        assert!(is_session_name("ae-x_1-9"));
        assert!(!is_session_name(""));
        assert!(!is_session_name("-lead"));
        assert!(!is_session_name(".dotproject"));
        assert!(!is_session_name("a/b"));
        assert!(!is_session_name(&"a".repeat(129)));
        assert!(is_session_name(&"a".repeat(128)));
    }

    #[test]
    fn a_direct_child_is_one_segment_below_its_parent() {
        assert!(is_direct_child("/s", "/s/name"));
        assert!(!is_direct_child("/s", "/s/a/b"));
        assert!(!is_direct_child("/s", "/s/.."));
        assert!(!is_direct_child("/s", "/other/name"));
    }
}
