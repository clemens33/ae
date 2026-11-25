//! The one notice per vendor block a session's lead gets (lead ruling 3).
//!
//! A block record is the fleet's ([`crate::vendorblock`]), and every session's
//! own watchdog tells its own lead, never another session's: the watchdog
//! that books a block tells its lead on the cycle that sees it, and every other
//! session's watchdog sees the same record on its own cycle. A session with no
//! running watchdog hears it on its next start, and its spawn and reseat
//! refusals name the block meanwhile.
//!
//! The receipt is the delivered `send` event itself ([`NOTICE_ACTION`], ref
//! `<identity>#<slot>`), so a later cycle and a restart find it and never
//! repeat the notice; a clear and a reblock make a new identity. Resume
//! retention keeps a receipt whose block still stands
//! (`session_launch::trim_events`). The remaining window: a crash after the
//! pane received the text and before the helper's event landed repeats the
//! notice once.

use super::{Cycle, SeatVerdict, Verdict};
use crate::events::Event;
use crate::vendorblock::{NOTICE_ACTION, Scope};

impl Cycle<'_> {
    /// Tell each lead seat of this session, once, about every block on a scope
    /// this session can launch on ([`reach`]). A seat that is dead or on a
    /// human prompt waits for a later cycle, and so does a delivery that did
    /// not land: only a delivered event is a receipt.
    pub(super) fn tell_blocks(&self, verdicts: &[(String, SeatVerdict)], events: &[Event], now: i64) {
        let Some(root) = self.meta_dir.parent().and_then(std::path::Path::parent) else {
            return;
        };
        let global = crate::doors::config_file(crate::shape::current(), root);
        let Ok(cfg) =
            crate::config::read_identity(Some(global.as_path()), self.local_config.as_deref())
        else {
            return;
        };
        for scope in reach(&cfg, &self.roster) {
            let Some(record) = crate::vendorblock::block(root, &scope, now) else {
                continue;
            };
            let identity = record.identity();
            for entry in self
                .roster
                .iter()
                .filter(|entry| super::in_lead_pair(&entry.slot, self.lead_pair))
            {
                let live = verdicts.iter().any(|(slot, seat)| {
                    *slot == entry.slot
                        && !matches!(seat.verdict, Verdict::Dead | Verdict::HumanPrompt)
                });
                let reference = format!("{identity}#{}", entry.slot);
                if !live || noticed(events, &reference) {
                    continue;
                }
                let summary = format!("vendor block notice: {}", record.stem());
                let _ = self.deliver(
                    &entry.name,
                    &record.notice(),
                    NOTICE_ACTION,
                    &summary,
                    Some(&reference),
                    Some(&entry.slot),
                );
            }
        }
    }
}

/// Whether the journal holds the delivered notice `reference` names.
fn noticed(events: &[Event], reference: &str) -> bool {
    events
        .iter()
        .any(|event| event.action == NOTICE_ACTION && event.reference.as_deref() == Some(reference))
}

/// The scopes this session can launch on, each judged by the ONE scope a
/// spawn of it would be in ([`Scope::of_profile`]): every configured profile
/// on its own client, and every seat of the roster on the client it records.
/// A profile that names no configured client reaches nothing.
pub(super) fn reach(
    cfg: &crate::config::IdentityConfig,
    roster: &[crate::meta::RosterEntry],
) -> Vec<Scope> {
    let mut scopes: Vec<Scope> = Vec::new();
    let seated = roster.iter().filter_map(|entry| {
        let profile = entry.profile.as_deref()?;
        let client = match &entry.client {
            crate::meta::RecordedClient::Label(label) => Some(label.as_str()),
            _ => None,
        };
        Scope::of_profile(cfg, profile, client)
    });
    let bare = cfg
        .profiles
        .iter()
        .filter_map(|(profile, _)| Scope::of_profile(cfg, profile, None));
    for scope in bare.chain(seated) {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    scopes
}

#[cfg(test)]
mod tests {
    use super::{noticed, reach};
    use crate::events::Event;
    use crate::meta::{
        RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, RecordedWorkDir, RosterEntry,
    };

    fn cfg() -> crate::config::IdentityConfig {
        crate::config::parse_identity(
            "[clients]\nopencode = opencode\nmuse = muse\n\n[profiles]\n\
             cop = opencode --model github-copilot/gpt-6.1-sol\n\
             zen = opencode --model opencode/big-pickle\n\
             bare = opencode --model glm-5\n",
        )
        .expect("config")
    }

    fn stems(scopes: &[crate::vendorblock::Scope]) -> Vec<String> {
        scopes.iter().map(crate::vendorblock::Scope::stem).collect()
    }

    #[test]
    fn reach_is_each_profiles_one_scope_and_no_bare_client_of_a_provider_only_session() {
        let cfg = crate::config::parse_identity(
            "[clients]\nopencode = opencode\n\n[profiles]\n\
             zen = opencode --model opencode/big-pickle\n",
        )
        .expect("config");
        assert_eq!(stems(&reach(&cfg, &[])), ["provider-opencode"]);
    }

    #[test]
    fn reach_adds_a_seat_on_another_client_through_its_recorded_label() {
        let roster = [RosterEntry {
            slot: "spawned.0".to_owned(),
            name: "w".to_owned(),
            profile: Some("bare".to_owned()),
            client: RecordedClient::Label("muse".to_owned()),
            harness_session: None,
            config_home: RecordedConfigHome::Missing,
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: None,
            work_dir: RecordedWorkDir::Missing,
        }];
        let scopes = stems(&reach(&cfg(), &roster));
        assert_eq!(
            scopes,
            [
                "provider-github-copilot",
                "provider-opencode",
                "client-opencode",
                "client-muse"
            ]
        );
    }

    #[test]
    fn only_a_delivered_notice_with_the_exact_reference_is_a_receipt() {
        let line = |action: &str, reference: &str| {
            Event::parse_line(&format!(
                r#"{{"ts":"2026-09-08T09:10:00Z","actor":"watchdog","action":"{action}","ref":"{reference}"}}"#
            ))
            .expect("event")
        };
        let events = [
            line("send", "client-muse@1#main"),
            line(crate::vendorblock::NOTICE_ACTION, "client-muse@1#worker.0"),
        ];
        assert!(!noticed(&events, "client-muse@1#main"));
        assert!(noticed(&events, "client-muse@1#worker.0"));
    }
}
