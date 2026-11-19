//! Session names: the grammar every explicit launch name must satisfy.
//!
//! Ported from `ae`'s `_validate_session_name`. The grammar is an ALLOWLIST
//! because a session name becomes a tmux session, a directory under
//! `~/.ae/sessions`, part of `.lifecycle.<name>.lock`, a neighbour in tmux
//! format strings, and the target of the launch rollback's recursive delete.

pub(crate) use crate::config::{SESSION_NAME_GRAMMAR, is_session_name};

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
    fn a_direct_child_is_one_segment_below_its_parent() {
        assert!(is_direct_child("/s", "/s/name"));
        assert!(!is_direct_child("/s", "/s/a/b"));
        assert!(!is_direct_child("/s", "/s/.."));
        assert!(!is_direct_child("/s", "/other/name"));
    }
}
