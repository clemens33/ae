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
        let reached =
            crate::config::read_identity(Some(global.as_path()), self.local_config.as_deref())
                .map(|cfg| {
                    let approved = crate::vendorblock::approved_scopes(&global);
                    reach(&cfg, &approved, &self.roster)
                })
                .unwrap_or_default();
        let mut records = written(events, root, now);
        for scope in &reached {
            if let Some(record) = crate::vendorblock::block(root, scope, now)
                && !records
                    .iter()
                    .any(|held| held.identity() == record.identity())
            {
                records.push(record);
            }
        }
        for record in &records {
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
                    self.claim_and_tell(entry, root, &identity, &reference);
                }
            }
        }
    }

    /// Deliver one notice under the session's notice lock, after reading the
    /// journal and the block again: a replacement watchdog, or a second one
    /// whose cycle began before the first receipt, finds that receipt and
    /// sends nothing, and a block cleared or replaced while this one waited is
    /// told by its own identity or not at all. A journal that cannot be read
    /// or classified defers the send: nothing then proves no receipt exists.
    /// The send helper never takes this lock, so holding it across the delivery
    /// cannot deadlock it. A lock that cannot be had sends nothing this cycle.
    fn claim_and_tell(
        &self,
        entry: &crate::meta::RosterEntry,
        root: &std::path::Path,
        identity: &str,
        reference: &str,
    ) {
        let lock = crate::store::lock_path(&self.meta_dir.join(NOTICE_ACTION));
        let Ok(_held) = crate::store::lock(&lock, crate::store::LOCK_WAIT) else {
            return;
        };
        let events = match crate::store::open(self.meta_dir).events_source() {
            crate::store::SourceRead::Absent => Vec::new(),
            crate::store::SourceRead::Ready(bytes) => crate::event_text::read_lines(&bytes)
                .into_iter()
                .filter_map(|line| Event::parse_line(&String::from_utf8_lossy(line)).ok())
                .collect(),
            crate::store::SourceRead::Invalid(_) | crate::store::SourceRead::Unreadable(_) => {
                return;
            }
        };
        if noticed(&events, reference) {
            return;
        }
        let now = crate::time::Timestamp::now().epoch();
        let Some(record) = crate::vendorblock::standing_identity(root, identity, now) else {
            return;
        };
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

/// The standing records this session's own seats drew, from the watchdog's
/// [`crate::vendorblock::WRITTEN_ACTION`] events: the own lead hears of these
/// whether or not a configured profile reaches the scope, and whether or not
/// the config reads at all. A cleared or replaced record is gone from here.
fn written(events: &[Event], root: &std::path::Path, now: i64) -> Vec<crate::vendorblock::Record> {
    let mut records: Vec<crate::vendorblock::Record> = Vec::new();
    for event in events.iter().filter(|event| {
        event.action == crate::vendorblock::WRITTEN_ACTION && event.actor == super::ACTOR
    }) {
        let Some(identity) = event.reference.as_deref() else {
            continue;
        };
        if let Some(record) = crate::vendorblock::standing_identity(root, identity, now)
            && !records.iter().any(|held| held.identity() == identity)
        {
            records.push(record);
        }
    }
    records
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
/// on its own client, every approved `profile@client` pair
/// ([`crate::vendorblock::approved_scopes`]), and every seat of the roster on
/// the client it records while its profile is still configured. A profile
/// that names no configured client, or a seat whose profile is gone, reaches
/// nothing: a removed profile proves no launch.
pub(super) fn reach(
    cfg: &crate::config::IdentityConfig,
    approved: &[Scope],
    roster: &[crate::meta::RosterEntry],
) -> Vec<Scope> {
    let bare = cfg
        .profiles
        .iter()
        .filter_map(|(profile, _)| Scope::of_profile(cfg, profile, None));
    let seated = roster.iter().filter_map(|entry| {
        let profile = entry.profile.as_deref()?;
        cfg.profile(profile)?;
        let client = match &entry.client {
            crate::meta::RecordedClient::Label(label) => Some(label.as_str()),
            _ => None,
        };
        Scope::of_profile(cfg, profile, client)
    });
    let mut scopes: Vec<Scope> = Vec::new();
    for scope in bare.chain(approved.iter().cloned()).chain(seated) {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    scopes
}

#[cfg(test)]
mod tests {
    use super::{noticed, reach, written};
    use crate::events::Event;
    use crate::meta::{
        RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, RecordedWorkDir, RosterEntry,
    };

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
        assert_eq!(stems(&reach(&cfg, &[], &[])), ["provider-opencode"]);
    }

    /// The approved pairs ride in as given; a seat adds the scope of the
    /// client it records, and nothing is listed twice.
    #[test]
    fn reach_adds_the_approved_scopes_and_each_recorded_seat_once() {
        let cfg = crate::config::parse_identity(
            "[clients]\nopencode = opencode\nocwork = opencode\n\n[profiles]\n\
             bare = opencode --model glm-5\n",
        )
        .expect("config");
        let approved = [crate::vendorblock::Scope::new(
            "ocwork",
            crate::tool::ToolKind::OpenCode,
            None,
        )];
        assert_eq!(
            stems(&reach(&cfg, &approved, &[])),
            ["client-opencode", "client-ocwork"]
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
            stems(&reach(&cfg, &[], &roster)),
            ["client-opencode", "client-ocwork"]
        );
        assert_eq!(
            stems(&reach(&cfg, &approved, &roster)),
            ["client-opencode", "client-ocwork"]
        );
        let gone = [RosterEntry {
            profile: Some("removed".to_owned()),
            ..roster[0].clone()
        }];
        assert_eq!(
            stems(&reach(&cfg, &[], &gone)),
            ["client-opencode"],
            "a seat whose profile is gone proves no launch on its recorded client"
        );
    }

    /// The blocks this session's own seat drew stand on their own, from the
    /// watchdog's events alone and with no config: a cleared or replaced
    /// record, another actor's line and a repeat add nothing.
    #[test]
    fn the_written_blocks_are_the_standing_records_the_own_seats_drew() {
        let root =
            std::env::temp_dir().join(format!("ae-blocknotice-written-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("root");
        let record = crate::vendorblock::parse(
            b"version=1\ntool=opencode\nclient=ocbin\nsince=1788858600\nkind=account\n\
              seat=w\nreason=You have exceeded your monthly quota\n",
        )
        .expect("record");
        assert_eq!(
            crate::vendorblock::record_first(&root, &record).ok(),
            Some(true)
        );
        let identity = record.identity();
        let line = |actor: &str, reference: &str| {
            Event::parse_line(&format!(
                r#"{{"ts":"2026-09-08T09:10:00Z","actor":"{actor}","action":"{}","ref":"{reference}"}}"#,
                crate::vendorblock::WRITTEN_ACTION
            ))
            .expect("event")
        };
        let stale = identity.replace("1788858600", "1788858599");
        let events = [
            line("watchdog", &identity),
            line("watchdog", &identity),
            line("lead", &identity),
            line("watchdog", &stale),
        ];
        let found = written(&events, &root, record.since);
        assert_eq!(
            found
                .iter()
                .map(crate::vendorblock::Record::identity)
                .collect::<Vec<_>>(),
            [identity.clone()]
        );
        assert!(written(&events[2..], &root, record.since).is_empty());
        std::fs::remove_file(root.join("vendorblock/client-ocbin.rec")).expect("clear");
        assert!(written(&events, &root, record.since).is_empty());
        let _ = std::fs::remove_dir_all(&root);
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
            line(
                "lead",
                crate::vendorblock::NOTICE_ACTION,
                "client-muse@1#spawned.0",
            ),
            line(
                "watchdog",
                crate::vendorblock::NOTICE_ACTION,
                "client-muse@1#worker.0",
            ),
        ];
        assert!(!noticed(&events, "client-muse@1#main"));
        assert!(
            !noticed(&events, "client-muse@1#spawned.0"),
            "another actor's line"
        );
        assert!(noticed(&events, "client-muse@1#worker.0"));
    }
}
