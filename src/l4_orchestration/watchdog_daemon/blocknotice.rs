//! The one notice per vendor block a session's lead gets (lead ruling 3).
//!
//! A block record is the fleet's ([`crate::vendorblock`]), and every session's
//! own watchdog tells its own lead, never another session's: the watchdog
//! that books a block tells its lead on the cycle that sees it, and every other
//! session's watchdog sees the same record on its own cycle. A session with no
//! running watchdog hears it on its next start, and its spawn and reseat
//! refusals name the block meanwhile.
//!
//! The receipt is the watchdog's own journal event ([`NOTICE_ACTION`], ref
//! `<identity>#<slot>`), written once the leg delivered or submitted the
//! notice, so a later cycle and a restart find it and never repeat it; a
//! clear and a reblock make a new identity. Resume
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
    /// not land: only a notice that was delivered, or submitted without a
    /// confirmation and so possibly delivered, leaves a receipt.
    ///
    /// `drawn` says a seat of this cycle drew a block: its
    /// [`crate::vendorblock::WRITTEN_ACTION`] row postdates the cycle's
    /// snapshot `events`, so the journal is read again and the own lead is
    /// told on this cycle, not the next ([`notice_source`]).
    pub(super) fn tell_blocks(
        &self,
        (verdicts, observed): (&[(String, SeatVerdict)], &[crate::tmux::WatchPane]),
        snapshot: &[Event],
        drawn: bool,
        now: i64,
    ) {
        let events = &*notice_source(snapshot, drawn, || super::read_events(self.meta_dir));
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
        let records = union(
            written(events, root, now),
            reached
                .iter()
                .filter_map(|scope| crate::vendorblock::block(root, scope, now)),
        );
        for record in &records {
            let identity = record.identity();
            for entry in self
                .roster
                .iter()
                .filter(|entry| super::in_lead_pair(&entry.slot, self.lead_pair))
            {
                let reference = format!("{identity}#{}", entry.slot);
                if due(verdicts, events, &entry.slot, &reference) {
                    self.claim_and_tell((entry, observed), root, &identity, &reference);
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
    /// The delivery never takes this lock, so holding it across the delivery
    /// cannot deadlock it. A lock that cannot be had sends nothing this cycle.
    fn claim_and_tell(
        &self,
        (entry, observed): (&crate::meta::RosterEntry, &[crate::tmux::WatchPane]),
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
        self.tell_lead((entry, observed), &record, reference);
    }

    /// The notice itself, as ae's own leg to the lead seat: the pane this
    /// cycle observed in the seat's slot, admitted in process against the
    /// launch this cycle's snapshot read (a legacy launch is the empty token),
    /// the way the charter update and the session notice reach a seat. The
    /// public send would refuse the orchestrator's main seat, whose initiation
    /// it cannot prove. A delivery, or a submit that may have reached the pane
    /// unconfirmed, journals the receipt; anything else tries again later.
    fn tell_lead(
        &self,
        (entry, observed): (&crate::meta::RosterEntry, &[crate::tmux::WatchPane]),
        record: &crate::vendorblock::Record,
        reference: &str,
    ) {
        let mut panes = observed
            .iter()
            .filter(|pane| pane.slot.as_deref() == Some(entry.slot.as_str()));
        let (Some(pane), None) = (panes.next(), panes.next()) else {
            return;
        };
        let launch = match self.launch_ids.iter().find(|(slot, _)| *slot == entry.slot) {
            None => "",
            Some((_, super::SeatLaunch::Id(id))) => id.as_str(),
            Some((_, super::SeatLaunch::Unproven(_))) => return,
        };
        let text = record.notice();
        let request = crate::deliver::Request {
            dir: self.meta_dir,
            server: self.server,
            pane: &pane.pane_id,
            logged_target: &entry.name,
            target_session: self.session,
            pane_slot: &entry.slot,
            own_session: self.session,
            action: NOTICE_ACTION,
            reference,
            actor: super::ACTOR,
            body: &text,
            shape: crate::deliver::Shape::Send,
            defer: NOTICE_DEFER,
            composed: crate::tool::Composed::NONE,
        };
        let mut err = std::io::stderr();
        let Ok(Some(guard)) = crate::tracked::admit_leg(&request, (&entry.slot, launch), &mut err)
        else {
            return;
        };
        let landed = matches!(
            crate::deliver::deliver_checked(&request, Some(&guard), false, &mut err),
            Ok(Ok(_) | Err(crate::deliver::Failure::Unconfirmed { notice: false, .. }))
        );
        if landed {
            let _ = super::Journal {
                meta_dir: self.meta_dir,
                session: self.session,
            }
            .record_routed(
                NOTICE_ACTION,
                (&entry.name, &entry.slot),
                reference,
                &format!("vendor block notice: {}", record.stem()),
                &mut err,
            );
        }
    }
}

/// How long a notice waits for a busy lead before a later cycle tries again.
const NOTICE_DEFER: std::time::Duration = std::time::Duration::from_secs(2);

/// The events a cycle's notice step reads: the cycle's snapshot, or when a
/// seat drew a block on this cycle the journal read again, so the row that
/// the draw appended is in it.
fn notice_source(
    snapshot: &[Event],
    drawn: bool,
    reread: impl FnOnce() -> Vec<Event>,
) -> std::borrow::Cow<'_, [Event]> {
    if drawn {
        std::borrow::Cow::Owned(reread())
    } else {
        std::borrow::Cow::Borrowed(snapshot)
    }
}

/// The blocks to tell: the own seats' written ones first, then each reached
/// block whose identity is not already listed, each identity once.
fn union(
    mut records: Vec<crate::vendorblock::Record>,
    reached: impl IntoIterator<Item = crate::vendorblock::Record>,
) -> Vec<crate::vendorblock::Record> {
    for record in reached {
        if !records
            .iter()
            .any(|held| held.identity() == record.identity())
        {
            records.push(record);
        }
    }
    records
}

/// The seat in `slot` can hear a notice this cycle: this cycle judged that
/// very seat, and it is neither dead nor on a prompt only the human answers.
/// Another seat's verdict says nothing about it.
fn can_hear(verdicts: &[(String, SeatVerdict)], slot: &str) -> bool {
    verdicts.iter().any(|(judged, seat)| {
        judged == slot && !matches!(seat.verdict, Verdict::Dead | Verdict::HumanPrompt)
    })
}

/// A notice `reference` is due to the seat in `slot` on this cycle's
/// snapshot: the seat can hear it ([`can_hear`]) and the snapshot holds no
/// receipt for it ([`noticed`]). The send rechecks the receipt under the
/// notice lock, for a concurrent watchdog.
fn due(verdicts: &[(String, SeatVerdict)], events: &[Event], slot: &str, reference: &str) -> bool {
    can_hear(verdicts, slot) && !noticed(events, reference)
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
    use super::{can_hear, due, notice_source, noticed, reach, union, written};
    use crate::events::Event;
    use crate::meta::{
        RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, RecordedWorkDir, RosterEntry,
    };

    fn record(client: &str, since: i64) -> crate::vendorblock::Record {
        crate::vendorblock::parse(
            format!(
                "version=1\ntool=muse\nclient={client}\nsince={since}\nkind=billing\n\
                 seat=ember\nreason=Billing verification failed\n"
            )
            .as_bytes(),
        )
        .expect("a record")
    }

    /// A written block and a reached block of another scope are both told;
    /// a reached duplicate of the written one is told once.
    #[test]
    fn the_blocks_told_are_the_written_ones_and_every_other_reached_one() {
        let told: Vec<String> = union(
            vec![record("muse", 1_788_858_600)],
            [
                record("muse", 1_788_858_600),
                record("ocwork", 1_788_858_700),
            ],
        )
        .iter()
        .map(crate::vendorblock::Record::identity)
        .collect();
        let stems: Vec<&str> = told
            .iter()
            .map(|identity| identity.split('@').next().unwrap_or_default())
            .collect();
        assert_eq!(stems, ["client-muse", "client-ocwork"], "{told:?}");
        assert!(told[0].starts_with("client-muse@1788858600/"), "{told:?}");
        assert!(told[1].starts_with("client-ocwork@1788858700/"), "{told:?}");
    }

    fn seat(verdict: super::Verdict) -> super::SeatVerdict {
        super::SeatVerdict {
            verdict,
            frame: crate::harness_state::HarnessState::default(),
            task: false,
        }
    }

    /// Only the slot's own verdict counts: a dead or prompting lead waits even
    /// while another seat of the session is live, and a live lead hears.
    #[test]
    fn only_the_leads_own_live_verdict_lets_it_hear_a_notice() {
        use super::Verdict::{Dead, HumanPrompt, Idle};
        for (lead, hears) in [(Dead, false), (HumanPrompt, false), (Idle, true)] {
            let verdicts = [
                ("worker.1".to_owned(), seat(Idle)),
                ("main".to_owned(), seat(lead)),
            ];
            assert_eq!(can_hear(&verdicts, "main"), hears, "{lead:?}");
        }
        assert!(
            !can_hear(&[("worker.1".to_owned(), seat(Idle))], "main"),
            "a seat this cycle never judged waits"
        );
    }

    /// A notice is due only to a lead that can hear it and has no receipt
    /// for it: a live lead with its receipt, and a dead or prompting lead
    /// without one, are both passed over.
    #[test]
    fn a_notice_is_due_only_to_a_live_lead_without_its_receipt() {
        use super::Verdict::{Dead, HumanPrompt, Idle};
        let receipts = [Event::parse_line(&format!(
            r#"{{"ts":"2026-09-08T09:10:00Z","actor":"watchdog","action":"{}","ref":"client-muse@1#main"}}"#,
            crate::vendorblock::NOTICE_ACTION
        ))
        .expect("receipt")];
        for (lead, reference, is_due) in [
            (Idle, "client-muse@1#main", false),
            (Idle, "client-muse@2#main", true),
            (Dead, "client-muse@2#main", false),
            (HumanPrompt, "client-muse@2#main", false),
        ] {
            let verdicts = [
                ("worker.1".to_owned(), seat(Idle)),
                ("main".to_owned(), seat(lead)),
            ];
            assert_eq!(
                due(&verdicts, &receipts, "main", reference),
                is_due,
                "{lead:?} {reference}"
            );
        }
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

    /// A cycle on which a seat drew a block reads the journal again, so the
    /// row the draw appended after the snapshot reaches this cycle's notice;
    /// any other cycle reads its snapshot and nothing more.
    #[test]
    fn a_cycle_that_drew_a_block_reads_the_row_it_appended() {
        let row = Event::parse_line(&format!(
            r#"{{"ts":"2026-09-08T09:10:00Z","actor":"watchdog","action":"{}","ref":"client-ocbin@1"}}"#,
            crate::vendorblock::WRITTEN_ACTION
        ))
        .expect("event");
        let snapshot: Vec<Event> = Vec::new();
        let drawn = notice_source(&snapshot, true, || vec![row.clone()]);
        assert_eq!(drawn.len(), 1, "the drawn row is read this cycle");
        let quiet = notice_source(&snapshot, false, || panic!("no reread"));
        assert!(quiet.is_empty());
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
            std::slice::from_ref(&identity)
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
