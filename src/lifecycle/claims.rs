//! Which OTHER sessions name a conversation id.
//!
//! `ae end --purge-history` removes the agent CLI's own conversation file for
//! every id the ended session records. An id is only a NAME, and a foreign one
//! — a capture that stored another session's rollout, a hand edit — would turn
//! that purge into the deletion of a conversation a different session still
//! resumes. So an id is removed only when no other session under the state
//! root names it; a session whose meta cannot be judged leaves every id
//! unproven, because absence cannot be shown from a file nobody could read.

use std::io::{self, Write};
use std::path::Path;

use crate::meta::{self, Meta};

use super::{census, sessions_dir};

/// What the rest of the state root says about one conversation id.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// No other session names it, and every other session was judged.
    Free,
    /// Another session names it: that session and the slot of its row.
    Named { session: String, slot: String },
    /// Some other session could not be judged, so nothing proves the id unshared.
    Unprovable(String),
}

impl Verdict {
    /// Whether the conversation file of `slot` must stay, saying why on `err`.
    ///
    /// The note names the other session and its slot, never a path.
    pub(super) fn keeps(&self, slot: &str, err: &mut impl Write) -> io::Result<bool> {
        match self {
            Self::Free => Ok(false),
            Self::Named {
                session,
                slot: theirs,
            } => {
                writeln!(
                    err,
                    "  note: conversation file for slot {slot} left in place (session {session} also names it: slot {theirs})"
                )?;
                Ok(true)
            }
            Self::Unprovable(why) => {
                writeln!(
                    err,
                    "  note: conversation file for slot {slot} left in place (cannot prove no other session names it: {why})"
                )?;
                Ok(true)
            }
        }
    }
}

/// The other sessions' metas, read once.
#[derive(Debug)]
pub(super) struct Others {
    /// Each other session whose meta could be read, with its
    /// [`meta::named_conversations`] as `(slot, id)`.
    judged: Vec<(String, Vec<(String, String)>)>,
    /// Why some other session could not be judged, first cause only.
    doubt: Option<String>,
}

impl Others {
    /// Read every session under `root` except `ended`.
    ///
    /// Every readable meta is kept, so a row that names an id counts whatever
    /// else is wrong with its file. A listing that fails, a session directory
    /// with no meta or an unreadable one, and a meta [`unjudgeable`] finds
    /// wanting are UNPROVABLE: nothing shows their rows name no id.
    pub(super) fn scan(root: &Path, ended: &str) -> Self {
        let mut others = Self {
            judged: Vec::new(),
            doubt: None,
        };
        let names = match census(root) {
            Ok(names) => names.unwrap_or_default(),
            Err(why) => {
                others.doubt = Some(format!("the sessions cannot be listed ({why})"));
                return others;
            }
        };
        let sessions = sessions_dir(root);
        for name in names.into_iter().filter(|name| name != ended) {
            let bytes = match meta::read_bytes(&sessions.join(&name)) {
                Ok(bytes) => bytes,
                Err(why) if why.kind() == io::ErrorKind::NotFound => {
                    others.doubt_session(&name, "missing");
                    continue;
                }
                Err(_) => {
                    others.doubt_session(&name, "unreadable");
                    continue;
                }
            };
            let state = match std::str::from_utf8(&bytes) {
                Ok(text) => unjudgeable(text),
                Err(_) => Some("not UTF-8"),
            };
            if let Some(state) = state {
                others.doubt_session(&name, state);
            }
            others
                .judged
                .push((name, meta::named_conversations(&bytes)));
        }
        others
    }

    fn doubt_session(&mut self, session: &str, state: &str) {
        self.doubt
            .get_or_insert_with(|| format!("session {session} meta is {state}"));
    }

    /// Whether `uuid` (canonical lowercase) is named by another session.
    ///
    /// Every conversation of every readable meta counts, whatever tool the other
    /// session runs and whether or not its roster claims the row, while a row that
    /// merely mentions the id (a goal, a path) does not. A named id outranks a
    /// doubt: it is the more exact reason.
    pub(super) fn verdict(&self, uuid: &str) -> Verdict {
        for (session, named) in &self.judged {
            if let Some((slot, _)) = named.iter().find(|(_, id)| id == uuid) {
                return Verdict::Named {
                    session: session.clone(),
                    slot: slot.clone(),
                };
            }
        }
        match &self.doubt {
            Some(why) => Verdict::Unprovable(why.clone()),
            None => Verdict::Free,
        }
    }
}

/// Why a UTF-8 meta cannot show that it names no conversation, if it cannot.
///
/// A parse with no anomaly proves nothing about a file that stops early: an
/// empty meta, one cut mid-row and one with no seat at all all parse clean. A
/// finished meta ends its last row and names the roster it describes, and a
/// damaged one is [`crate::session::anomalies_degrade`]'s to name — `UnknownKey`
/// is not damage, and every live meta carries one.
fn unjudgeable(text: &str) -> Option<&'static str> {
    if !text.ends_with('\n') {
        return Some("truncated");
    }
    let parsed = Meta::parse(text);
    if crate::session::anomalies_degrade(parsed.anomalies()) {
        return Some("damaged");
    }
    parsed.roster().is_empty().then_some("without a roster")
}

#[cfg(test)]
mod tests {
    use super::{Others, Verdict, unjudgeable};

    const ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";
    const NEIGHBOR: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

    fn scratch(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("ae-claims-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sessions")).expect("sessions");
        root
    }

    fn session(root: &std::path::Path, name: &str, meta: &[u8]) {
        let dir = root.join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("session dir");
        std::fs::write(dir.join("meta"), meta).expect("meta");
    }

    #[test]
    fn a_row_in_another_session_names_the_id_whatever_its_tool_or_case() {
        let root = scratch("named");
        session(
            &root,
            "other",
            format!(
                "schema=2\nseat.worker.3=n\nharness_session_prior.worker.3=codex:{NEIGHBOR},{}\n",
                ID.to_uppercase()
            )
            .as_bytes(),
        );
        let verdict = Others::scan(&root, "ended").verdict(ID);
        assert_eq!(
            verdict,
            Verdict::Named {
                session: "other".to_owned(),
                slot: "worker.3".to_owned()
            }
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_orphan_row_without_a_seat_still_names_the_id() {
        let root = scratch("orphan");
        session(
            &root,
            "other",
            format!("schema=2\nharness_session.spawned.9={ID}\n").as_bytes(),
        );
        assert!(matches!(
            Others::scan(&root, "ended").verdict(ID),
            Verdict::Named { slot, .. } if slot == "spawned.9"
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_ended_session_and_unrelated_ids_do_not_count() {
        let root = scratch("free");
        session(
            &root,
            "ended",
            format!("schema=2\nharness_session.main={ID}\n").as_bytes(),
        );
        session(
            &root,
            "other",
            b"schema=2\nseat.main=n\nunknown_future_key=1\nharness_session.main=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n",
        );
        assert_eq!(Others::scan(&root, "ended").verdict(ID), Verdict::Free);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_state_root_with_no_sessions_has_no_other_owner() {
        let empty = std::env::temp_dir().join(format!("ae-claims-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&empty);
        assert_eq!(Others::scan(&empty, "ended").verdict(ID), Verdict::Free);
    }

    #[test]
    fn a_meta_that_cannot_be_judged_leaves_the_id_unproven() {
        for (tag, damage, word) in [
            ("nonutf8", &[0xff, 0xfe][..], "not UTF-8"),
            ("noequals", &b"broken row\n"[..], "damaged"),
            (
                "duplicate",
                &b"schema=2\nseat.main=n\nharness_session.main=a\nharness_session.main=b\n"[..],
                "damaged",
            ),
        ] {
            let root = scratch(tag);
            session(&root, "other", damage);
            match Others::scan(&root, "ended").verdict(ID) {
                Verdict::Unprovable(why) => {
                    assert!(why.contains("session other"), "{why}");
                    assert!(why.contains(word), "{why}");
                }
                other => panic!("{tag}: {other:?}"),
            }
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn a_session_with_no_meta_or_an_unreadable_one_leaves_the_id_unproven() {
        let root = scratch("missing");
        std::fs::create_dir_all(root.join("sessions/bare")).expect("bare session");
        session(&root, "ok", b"schema=2\nseat.main=n\n");
        match Others::scan(&root, "ended").verdict(ID) {
            Verdict::Unprovable(why) => {
                assert!(why.contains("session bare meta is missing"), "{why}");
            }
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);

        let root = scratch("directory");
        std::fs::create_dir_all(root.join("sessions/odd/meta")).expect("meta as a directory");
        match Others::scan(&root, "ended").verdict(ID) {
            Verdict::Unprovable(why) => {
                assert!(why.contains("session odd meta is unreadable"), "{why}");
            }
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_named_id_outranks_a_doubt() {
        let root = scratch("both");
        session(&root, "aaa-broken", b"broken row\n");
        session(
            &root,
            "zzz-owner",
            format!("schema=2\nharness_session.main={ID}\n").as_bytes(),
        );
        assert!(matches!(
            Others::scan(&root, "ended").verdict(ID),
            Verdict::Named { session, .. } if session == "zzz-owner"
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_cut_or_seatless_meta_parses_clean_and_is_still_unprovable() {
        for (meta, state) in [
            ("", "truncated"),
            ("schema=2\nseat.main=n", "truncated"),
            ("schema=2\nseat.main=n\nharness_session.main=", "truncated"),
            ("broken row\n", "damaged"),
            ("schema=2\nsession=s\n", "without a roster"),
        ] {
            assert_eq!(unjudgeable(meta), Some(state), "{meta:?}");
        }
        // Unknown keys are not damage: every live meta carries one.
        assert_eq!(unjudgeable("schema=2\nseat.main=n\nfuture_key=1\n"), None);
    }

    #[test]
    fn a_seatless_or_unreadable_meta_still_names_an_id_it_spells() {
        let root = scratch("raw");
        session(
            &root,
            "orphan",
            format!("harness_session.x.1={ID}\n").as_bytes(),
        );
        assert!(matches!(
            Others::scan(&root, "ended").verdict(ID),
            Verdict::Named { session, slot } if session == "orphan" && slot == "x.1"
        ));
        let _ = std::fs::remove_dir_all(&root);

        let root = scratch("lossy");
        let mut bytes = format!("schema=2\nseat.main=n\nharness_session.main={ID}\n").into_bytes();
        bytes.push(0xff);
        session(&root, "lossy", &bytes);
        assert!(matches!(
            Others::scan(&root, "ended").verdict(ID),
            Verdict::Named { session, .. } if session == "lossy"
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_a_free_id_is_removed_and_a_kept_one_says_who_and_why() {
        let mut err = Vec::new();
        assert!(!Verdict::Free.keeps("main", &mut err).expect("write"));
        assert!(err.is_empty());
        let named = Verdict::Named {
            session: "other".to_owned(),
            slot: "worker.3".to_owned(),
        };
        assert!(named.keeps("main", &mut err).expect("write"));
        assert_eq!(
            String::from_utf8_lossy(&err),
            "  note: conversation file for slot main left in place (session other also names it: slot worker.3)\n"
        );
        err.clear();
        let unproven = Verdict::Unprovable("session x meta is damaged".to_owned());
        assert!(unproven.keeps("worker.1", &mut err).expect("write"));
        assert_eq!(
            String::from_utf8_lossy(&err),
            "  note: conversation file for slot worker.1 left in place (cannot prove no other session names it: session x meta is damaged)\n"
        );
    }

    #[test]
    fn an_unrelated_row_that_mentions_the_id_does_not_make_it_shared() {
        let root = scratch("goal");
        session(
            &root,
            "other",
            format!("schema=2\nseat.main=n\ngoal=revive {ID}\nsession_id={ID}\nwork_dir=/w/{ID}\n")
                .as_bytes(),
        );
        assert_eq!(Others::scan(&root, "ended").verdict(ID), Verdict::Free);
        let _ = std::fs::remove_dir_all(&root);
    }
}
