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
    pub(super) fn tell_blocks(
        &self,
        verdicts: &[(String, SeatVerdict)],
        events: &[Event],
        now: i64,
    ) {
        let Some(root) = self.meta_dir.parent().and_then(std::path::Path::parent) else {
            return;
        };
        let global = crate::doors::config_file(crate::shape::current(), root);
        let Ok(cfg) =
            crate::config::read_identity(Some(global.as_path()), self.local_config.as_deref())
        else {
            return;
        };
        // The approved `profile@client` pairs come from the global file alone,
        // as a spawn reads them: a project overlay approves nothing.
        let pairs = crate::config::read_selected(&global)
            .map(|text| crate::pairs::parse_in(&global, &text))
            .unwrap_or_default();
        for scope in reach(&cfg, &pairs, &self.roster) {
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
                if live && !noticed(events, &reference) {
                    self.claim_and_tell(entry, &record, &reference);
                }
            }
        }
    }

    /// Deliver one notice under the session's notice lock, after reading the
    /// journal again: a replacement watchdog, or a second one whose cycle began
    /// before the first receipt, finds that receipt and sends nothing. The
    /// send helper never takes this lock, so holding it across the delivery
    /// cannot deadlock it. A lock that cannot be had sends nothing this cycle.
    fn claim_and_tell(
        &self,
        entry: &crate::meta::RosterEntry,
        record: &crate::vendorblock::Record,
        reference: &str,
    ) {
        let lock = crate::store::lock_path(&self.meta_dir.join(NOTICE_ACTION));
        let Ok(_held) = crate::store::lock(&lock, crate::store::LOCK_WAIT) else {
            return;
        };
        if noticed(&super::read_events(self.meta_dir), reference) {
            return;
        }
        let summary = format!("vendor block notice: {}", record.stem());
        let _ = self.deliver(
            &entry.name,
            &record.notice(),
            NOTICE_ACTION,
            &summary,
            Some(reference),
            Some(&entry.slot),
        );
    }
}

/// Whether the journal holds the watchdog's notice `reference` names, whether
/// its submit was confirmed or not: an unconfirmed one may have reached the
/// pane, and a second copy is the repeat the receipt exists to prevent.
fn noticed(events: &[Event], reference: &str) -> bool {
    events.iter().any(|event| {
        event.action == NOTICE_ACTION
            && event.actor == super::ACTOR
            && event.reference.as_deref() == Some(reference)
    })
}

/// The scopes this session can launch on, each judged by the ONE scope a
/// spawn of it would be in ([`Scope::of_profile`]): every configured profile
/// on its own client, every `profile@client` pair the global `[client_pairs]`
/// approves whose client runs the profile's own adapter, and every seat of
/// the roster on the client it records. A profile that names no configured
/// client reaches nothing.
pub(super) fn reach(
    cfg: &crate::config::IdentityConfig,
    pairs: &crate::pairs::ApprovedPairs,
    roster: &[crate::meta::RosterEntry],
) -> Vec<Scope> {
    let tool_of = |label: &str| cfg.client(label).map(|client| client.tool);
    let bare = cfg
        .profiles
        .iter()
        .filter_map(|(profile, _)| Scope::of_profile(cfg, profile, None));
    let approved = cfg.profiles.iter().flat_map(|(profile, _)| {
        let own = cfg
            .profile_client_label(profile)
            .and_then(|label| tool_of(&label));
        cfg.clients.iter().filter_map(move |(label, client)| {
            let same = own == Some(client.tool);
            let listed = pairs.judge(profile, label) == crate::pairs::Verdict::Approved;
            (same && listed)
                .then(|| Scope::of_profile(cfg, profile, Some(label)))
                .flatten()
        })
    });
    let seated = roster.iter().filter_map(|entry| {
        let profile = entry.profile.as_deref()?;
        let client = match &entry.client {
            crate::meta::RecordedClient::Label(label) => Some(label.as_str()),
            _ => None,
        };
        Scope::of_profile(cfg, profile, client)
    });
    let mut scopes: Vec<Scope> = Vec::new();
    for scope in bare.chain(approved).chain(seated) {
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

    fn stems(scopes: &[crate::vendorblock::Scope]) -> Vec<String> {
        scopes.iter().map(crate::vendorblock::Scope::stem).collect()
    }

    fn pairs(text: &str) -> crate::pairs::ApprovedPairs {
        crate::pairs::parse_in(std::path::Path::new("/cfg"), text)
    }

    #[test]
    fn reach_is_each_profiles_one_scope_and_no_bare_client_of_a_provider_only_session() {
        let cfg = crate::config::parse_identity(
            "[clients]\nopencode = opencode\n\n[profiles]\n\
             zen = opencode --model opencode/big-pickle\n",
        )
        .expect("config");
        assert_eq!(
            stems(&reach(&cfg, &pairs(""), &[])),
            ["provider-opencode"]
        );
    }

    /// Two `OpenCode` clients: an approved pair adds the scope the pair's
    /// launch is in before any seat ran on it, an unapproved pair and a pair
    /// onto another adapter add nothing, and a recorded seat adds its own.
    #[test]
    fn reach_adds_approved_same_adapter_pairs_and_recorded_seats() {
        let cfg = crate::config::parse_identity(
            "[clients]\nopencode = opencode\nocwork = opencode\nmuse = muse\n\n[profiles]\n\
             bare = opencode --model glm-5\n\
             mus = muse --model muse-ember-1.0\n",
        )
        .expect("config");
        let approved = pairs("[client_pairs]\nbare = ocwork, muse\n");
        assert_eq!(
            stems(&reach(&cfg, &approved, &[])),
            ["client-opencode", "client-muse", "client-ocwork"]
        );
        assert_eq!(
            stems(&reach(&cfg, &pairs(""), &[])),
            ["client-opencode", "client-muse"]
        );
        let roster = [RosterEntry {
            slot: "spawned.0".to_owned(),
            name: "w".to_owned(),
            profile: Some("bare".to_owned()),
            client: RecordedClient::Label("ocwork".to_owned()),
            harness_session: None,
            config_home: RecordedConfigHome::Missing,
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: None,
            work_dir: RecordedWorkDir::Missing,
        }];
        assert_eq!(
            stems(&reach(&cfg, &pairs(""), &roster)),
            ["client-opencode", "client-muse", "client-ocwork"]
        );
    }

    #[test]
    fn only_a_delivered_notice_with_the_exact_reference_is_a_receipt() {
        let line = |actor: &str, action: &str, reference: &str| {
            Event::parse_line(&format!(
                r#"{{"ts":"2026-09-08T09:10:00Z","actor":"{actor}","action":"{action}","ref":"{reference}"}}"#
            ))
            .expect("event")
        };
        let events = [
            line("watchdog", "send", "client-muse@1#main"),
            line("lead", crate::vendorblock::NOTICE_ACTION, "client-muse@1#spawned.0"),
            line("watchdog", crate::vendorblock::NOTICE_ACTION, "client-muse@1#worker.0"),
        ];
        assert!(!noticed(&events, "client-muse@1#main"));
        assert!(!noticed(&events, "client-muse@1#spawned.0"), "another actor's line");
        assert!(noticed(&events, "client-muse@1#worker.0"));
    }
}
