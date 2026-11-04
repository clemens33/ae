//! The approved `profile@client` pair list: `[client_pairs]` in the GLOBAL
//! config, the explicit permission an agent-invoked `spawn --using
//! profile@client` needs before it may run a profile on another account.
//!
//! ```text
//! [client_pairs]
//! opus55x = cc-mic, cc-p   # profile = the clients it may run on
//! ```
//!
//! Policy lives here and nowhere else: written policy (the `spawn_instructions`
//! prose) is never translated into a permission, and a launch the human types
//! (`ae <name> --lead x@y`) is not gated by this list at all.
//!
//! The list fails CLOSED, row by row:
//!
//! - only the global file is read, so a project `.ae/config` can neither
//!   approve a pair nor redefine an approved name;
//! - a row ae cannot read in full approves NOTHING for its profile: an empty
//!   value, an empty entry (`a,,b`, a trailing comma) or an entry that is not
//!   a client label voids the whole row, never a part of it;
//! - a profile keyed twice voids its row, because "the later row wins" would
//!   let an ambiguity approve;
//! - an entry for a profile or client that does not exist is inert.
//!
//! A duplicate label inside one list is harmless and collapses. Matching is
//! exact and case-sensitive; whitespace around an entry is trimmed.

use std::path::Path;

use crate::config;

/// The config section the list is read from.
const SECTION: &str = "client_pairs";

/// The longest echo of a rejected entry inside a refusal.
const ECHO: usize = 40;

/// What one profile's row says.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// The clients the profile may run on, in written order, repeats collapsed.
    Usable(Vec<String>),
    /// A row that cannot be read in full: it approves nothing. The reason is
    /// worded for the refusal.
    Void(String),
}

/// The parsed list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApprovedPairs {
    /// One entry per distinct profile key, in file order.
    rows: Vec<(String, Row)>,
    /// Why the whole section could not be read, if it could not.
    unreadable: Option<String>,
}

/// The answer for one `profile@client` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The pair is on the list.
    Approved,
    /// The pair is not on the list; `approved` is what the profile IS allowed,
    /// empty when it has no usable row.
    Unlisted {
        /// The profile's approved clients, in written order.
        approved: Vec<String>,
    },
    /// The profile's row cannot be read in full.
    Void {
        /// The profile whose row is void.
        profile: String,
        /// Why, worded for the refusal.
        why: String,
    },
    /// The section as a whole could not be read.
    Unreadable(String),
}

impl Verdict {
    /// The refusal sentence for a pair that is not approved, `None` when it is.
    ///
    /// It names where the human approves a pair, and what the profile is
    /// already allowed when it has a usable row: an agent learns the allowed
    /// spellings from its refusal. The caller wraps it in its own verb.
    #[must_use]
    pub fn refusal(&self, profile: &str, global: &Path) -> Option<String> {
        match self {
            Self::Approved => None,
            Self::Unlisted { approved } => {
                let allowed = if approved.is_empty() {
                    String::new()
                } else {
                    format!(
                        " Approved clients for '{profile}': {}.",
                        approved.join(", ")
                    )
                };
                Some(format!(
                    "is not an approved pair — the human approves pairs under [client_pairs] in {}.{allowed}",
                    global.display()
                ))
            }
            Self::Void { profile, why } => Some(format!(
                "is not an approved pair — [client_pairs] '{profile}' is unusable ({why})."
            )),
            Self::Unreadable(why) => Some(format!(
                "is not an approved pair — [client_pairs] in {} cannot be read ({why}).",
                global.display()
            )),
        }
    }
}

impl ApprovedPairs {
    /// Judge one pair against the list.
    #[must_use]
    pub fn judge(&self, profile: &str, client: &str) -> Verdict {
        if let Some(why) = &self.unreadable {
            return Verdict::Unreadable(why.clone());
        }
        match self.rows.iter().find(|(key, _)| key == profile) {
            None => Verdict::Unlisted {
                approved: Vec::new(),
            },
            Some((_, Row::Void(why))) => Verdict::Void {
                profile: profile.to_owned(),
                why: why.clone(),
            },
            Some((_, Row::Usable(clients))) if clients.iter().any(|label| label == client) => {
                Verdict::Approved
            }
            Some((_, Row::Usable(clients))) => Verdict::Unlisted {
                approved: clients.clone(),
            },
        }
    }
}

/// An entry echoed into a refusal: escaped and clipped, because it came out of
/// a hand-edited file.
fn echo(entry: &str) -> String {
    let clipped: String = entry.chars().take(ECHO).collect();
    format!("{clipped:?}")
}

/// One value, as a row.
fn row_of(value: Option<&str>) -> Row {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Row::Void("the list is empty".to_owned());
    };
    let mut clients: Vec<String> = Vec::new();
    for item in value.split(',') {
        let item = item.trim();
        if item.is_empty() {
            return Row::Void("the list has an empty entry".to_owned());
        }
        // The grammar a `[clients]` key and a recorded `client.<slot>` value
        // use, not the 32-cell status-cell cap of `is_client_label`: a legal
        // 33-64 character client can launch with `@`, so it can be approved.
        if !config::is_agent_name(item) {
            return Row::Void(format!("{} is not a client label", echo(item)));
        }
        if !clients.iter().any(|label| label == item) {
            clients.push(item.to_owned());
        }
    }
    Row::Usable(clients)
}

/// Parse the list out of config text already read; `file` names the text in a
/// refusal. The pure entry the fuzz target and the unit tests share.
#[must_use]
pub fn parse_in(file: &Path, text: &str) -> ApprovedPairs {
    let entries = match config::section_entries(file, text, SECTION) {
        Ok(entries) => entries,
        Err(why) => {
            return ApprovedPairs {
                rows: Vec::new(),
                unreadable: Some(why),
            };
        }
    };
    let mut rows: Vec<(String, Row)> = Vec::new();
    for entry in entries {
        // A key no profile can carry is inert, not an error.
        if !config::is_config_key(&entry.key) {
            continue;
        }
        let row = row_of(entry.value.as_deref());
        match rows.iter_mut().find(|(key, _)| *key == entry.key) {
            Some((_, existing)) => {
                *existing = Row::Void(format!("'{}' is keyed more than once", entry.key));
            }
            None => rows.push((entry.key, row)),
        }
    }
    ApprovedPairs {
        rows,
        unreadable: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ApprovedPairs, Verdict, parse_in};

    fn pairs(body: &str) -> ApprovedPairs {
        parse_in(
            Path::new("config"),
            &format!("[client_pairs]\n{body}\n[workspace]\nmain = lead\n"),
        )
    }

    fn unlisted(approved: &[&str]) -> Verdict {
        Verdict::Unlisted {
            approved: approved.iter().map(|label| (*label).to_owned()).collect(),
        }
    }

    #[test]
    fn a_listed_pair_is_approved_and_an_unlisted_one_names_what_is() {
        let list = pairs("opus = cc-mic, cc-p\n");
        assert_eq!(list.judge("opus", "cc-mic"), Verdict::Approved);
        assert_eq!(list.judge("opus", "cc-p"), Verdict::Approved);
        assert_eq!(list.judge("opus", "cc"), unlisted(&["cc-mic", "cc-p"]));
        assert_eq!(list.judge("other", "cc-mic"), unlisted(&[]));
    }

    #[test]
    fn matching_is_exact_and_case_sensitive() {
        let list = pairs("opus = cc-mic\n");
        assert_eq!(list.judge("Opus", "cc-mic"), unlisted(&[]));
        assert_eq!(list.judge("opus", "CC-MIC"), unlisted(&["cc-mic"]));
    }

    #[test]
    fn whitespace_around_entries_is_trimmed_and_repeats_collapse() {
        let list = pairs("opus =  cc-mic ,cc-p,  cc-mic\n");
        assert_eq!(list.judge("opus", "cc-p"), Verdict::Approved);
        assert_eq!(
            list.judge("opus", "nope"),
            unlisted(&["cc-mic", "cc-p"]),
            "the repeated label collapsed"
        );
    }

    #[test]
    fn a_comment_after_the_list_is_not_part_of_it() {
        let list = pairs("opus = cc-mic   # the mic login\n");
        assert_eq!(list.judge("opus", "cc-mic"), Verdict::Approved);
    }

    #[test]
    fn a_row_ae_cannot_read_in_full_approves_nothing() {
        for body in [
            "opus =\n",
            "opus = cc-mic,\n",
            "opus = ,cc-mic\n",
            "opus = cc-mic,,cc-p\n",
            "opus = cc-mic, cc mic\n",
            "opus = cc-mic, -bad\n",
        ] {
            let list = pairs(body);
            assert!(
                matches!(list.judge("opus", "cc-mic"), Verdict::Void { .. }),
                "{body:?} must void the whole row: {:?}",
                list.judge("opus", "cc-mic")
            );
        }
    }

    #[test]
    fn a_client_label_longer_than_the_status_cell_cap_is_still_approvable() {
        let long = format!("c{}", "x".repeat(32));
        assert_eq!(long.len(), 33);
        let list = pairs(&format!("opus = {long}\n"));
        assert_eq!(list.judge("opus", &long), Verdict::Approved);
        let longest = format!("c{}", "x".repeat(63));
        assert_eq!(longest.len(), 64);
        assert_eq!(
            pairs(&format!("opus = {longest}\n")).judge("opus", &longest),
            Verdict::Approved
        );
        let too_long = format!("c{}", "x".repeat(64));
        assert!(matches!(
            pairs(&format!("opus = {too_long}\n")).judge("opus", &too_long),
            Verdict::Void { .. }
        ));
    }

    #[test]
    fn a_profile_keyed_twice_is_void_not_last_wins() {
        let list = pairs("opus = cc-mic\nopus = cc-p\n");
        assert!(matches!(list.judge("opus", "cc-p"), Verdict::Void { .. }));
        assert!(matches!(list.judge("opus", "cc-mic"), Verdict::Void { .. }));
    }

    #[test]
    fn a_key_repeated_across_two_headers_is_a_duplicate_too() {
        let list = parse_in(
            Path::new("config"),
            "[client_pairs]\nopus = cc-mic\n[workspace]\nmain = lead\n[client_pairs]\nopus = cc-p\n",
        );
        assert!(matches!(list.judge("opus", "cc-mic"), Verdict::Void { .. }));
    }

    #[test]
    fn a_void_row_does_not_touch_other_profiles() {
        let list = pairs("opus = cc-mic,,cc-p\nfable = cc-p\n");
        assert_eq!(list.judge("fable", "cc-p"), Verdict::Approved);
    }

    #[test]
    fn a_key_no_profile_can_carry_is_inert() {
        let list = pairs("a@b = cc-mic\nopus = cc-mic\n");
        assert_eq!(list.judge("opus", "cc-mic"), Verdict::Approved);
        assert_eq!(list.judge("a@b", "cc-mic"), unlisted(&[]));
    }

    #[test]
    fn other_sections_and_absence_approve_nothing() {
        let other = parse_in(
            Path::new("config"),
            "[auto_reseat]\nopus = cc-mic\n[profiles]\nopus = \"claude\"\n",
        );
        assert_eq!(other.judge("opus", "cc-mic"), unlisted(&[]));
        assert_eq!(
            ApprovedPairs::default().judge("opus", "cc-mic"),
            unlisted(&[])
        );
    }

    #[test]
    fn a_refusal_names_where_the_human_approves_and_what_is_allowed() {
        let global = Path::new("/h/.ae/config");
        let list = pairs("opus = cc-mic, cc-p\n");
        let line = list
            .judge("opus", "cc")
            .refusal("opus", global)
            .expect("an unlisted pair refuses");
        assert!(line.contains("is not an approved pair"), "{line}");
        assert!(line.contains("[client_pairs] in /h/.ae/config"), "{line}");
        assert!(
            line.contains("Approved clients for 'opus': cc-mic, cc-p."),
            "{line}"
        );
        let bare = list
            .judge("fable", "cc")
            .refusal("fable", global)
            .expect("refuses");
        assert!(!bare.contains("Approved clients"), "{bare}");
        assert_eq!(list.judge("opus", "cc-p").refusal("opus", global), None);
    }

    #[test]
    fn a_void_refusal_says_why_and_clips_a_hostile_entry() {
        let global = Path::new("/h/.ae/config");
        let hostile = format!("opus = cc-mic, {}\n", "x".repeat(200));
        let line = pairs(&hostile)
            .judge("opus", "cc-mic")
            .refusal("opus", global)
            .expect("void refuses");
        assert!(line.contains("[client_pairs] 'opus' is unusable"), "{line}");
        assert!(line.len() < 200, "the echo is clipped: {line}");
    }

    #[test]
    fn a_section_that_cannot_be_read_approves_nothing() {
        let list = parse_in(
            Path::new("config"),
            "[prompt]\ninstructions = \"\"\"\nnever closed\n[client_pairs]\nopus = cc-mic\n",
        );
        assert!(matches!(
            list.judge("opus", "cc-mic"),
            Verdict::Unreadable(_)
        ));
    }
}
