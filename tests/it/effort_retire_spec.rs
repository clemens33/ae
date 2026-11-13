//! P2 acceptance: retiring a spawned seat cannot silently lose applied effort.
//! Oracle: lead P2 ruling, item 4. These fixtures retain the v1 contract.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect process-owned scratch directories"
)]

use std::cell::Cell;
use std::path::PathBuf;

use ae::events::{Event, SessionKey};

const SID: &str = "11111111-1111-4111-8111-111111111111";
const EP: &str = "1700000000";
const VALID: &str = "v1 applied slot:spawned.0 base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:scout tool:codex launch:old-token pid:42 conv:22222222-2222-4222-8222-222222222222 profile:custom client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700000100";
const OTHER: &str = "seat.main=lead\nprofile.main=base\neffort.spawned.01=neighbor\ngoal=keep\n";

struct Rig(super::cli::OwnedScratch);

impl Rig {
    fn new(tag: &str) -> Self {
        let rig = Self(super::cli::OwnedScratch::root("efr", tag));
        std::fs::create_dir_all(rig.dir()).expect("owned session directory");
        std::fs::write(
            rig.dir().join("meta"),
            format!("session=fixture\nsession_id={SID}\n{OTHER}seat.spawned.0=scout\nprofile.spawned.0=custom\nagent_bin.spawned.0=codex\nlaunch_id.spawned.0=old-token\neffort.spawned.0={VALID}\n"),
        ).expect("literal applied spawned seat");
        std::fs::write(rig.dir().join("events.jsonl"), trace()).expect("literal applied history");
        rig
    }

    fn dir(&self) -> PathBuf {
        self.0.join("fixture")
    }

    fn meta(&self) -> String {
        std::fs::read_to_string(self.dir().join("meta")).expect("fixture metadata")
    }

    fn journal(&self) -> String {
        std::fs::read_to_string(self.dir().join("events.jsonl")).expect("fixture journal")
    }

    fn events(&self) -> Vec<Event> {
        let bytes = self.journal();
        assert!(
            bytes.ends_with('\n'),
            "only complete journal records authorize loss"
        );
        bytes
            .lines()
            .map(|line| Event::parse_line(line).expect("valid journal event"))
            .collect()
    }

    fn remove(&self, proven: bool, cleaned: &Cell<bool>) -> Result<String, String> {
        if proven {
            let proof = ae::identity::prove_removable(&self.dir(), "scout")?;
            ae::identity::remove_proven_slot(&self.dir(), "scout", &proof, || cleaned.set(true))
                .map_err(|why| format!("{why:?}"))
        } else {
            ae::identity::remove_seat_slot(&self.dir(), "scout")
        }
    }

    fn assert_closed(&self) {
        self.assert_closed_ref(EP);
    }

    fn assert_closed_ref(&self, reference: &str) {
        let events = self.events();
        let endings: Vec<_> = events
            .iter()
            .filter(|event| {
                matches!(
                    event.action.as_str(),
                    "effort-discarded" | "effort-restored"
                )
            })
            .collect();
        assert_eq!(
            endings.len(),
            1,
            "one ending for the applied adjustment: {events:?}"
        );
        let ending = endings[0];
        assert_eq!(
            ending.action, "effort-discarded",
            "retirement does not restore a tool"
        );
        assert_eq!(ending.actor, "watchdog");
        assert_eq!(ending.reference.as_deref().unwrap_or_default(), reference);
        assert!(
            ending
                .summary
                .as_deref()
                .is_some_and(|reason| !reason.is_empty())
        );
        assert!(
            SessionKey::new("fixture", SID)
                .seat("spawned.0")
                .matches(ending.target_identity().expect("routed ending"))
        );
        assert_eq!(
            ae::effort::fold(
                &events,
                SessionKey::new("fixture", SID),
                "spawned.0",
                "scout"
            ),
            ae::effort::Fold::Armed,
            "the actual effort fold accepts the ending"
        );
    }
}

fn trace() -> String {
    [
        ("effort-episode", EP, "threshold 80"),
        ("effort-reserved", "1700000000-down", "xhigh->high"),
        ("effort-applied", "1700000000-down", "xhigh->high"),
    ].map(|(action, reference, summary)| format!("{{\"ts\":\"2023-11-14T22:13:20Z\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session\":\"fixture\",\"target_session_id\":\"{SID}\",\"ref\":\"{reference}\",\"summary\":\"{summary}\"}}\n")).concat()
}

#[test]
fn effort_retire_fixture_is_a_valid_v1_applied_spawned_episode() {
    let rig = Rig::new("premise");
    assert!(
        matches!(ae::effort::stored(rig.meta().as_bytes(), "spawned.0"), ae::effort::Stored::Present(record) if record == ae::effort::Record::parse(VALID).expect("literal valid record"))
    );
    assert!(
        matches!(ae::effort::fold(&rig.events(), SessionKey::new("fixture", SID), "spawned.0", "scout"), ae::effort::Fold::Open(episode) if matches!(episode.down, ae::effort::Leg::Done(_)))
    );
    assert_eq!(
        ae::identity::prove_removable(&rig.dir(), "scout")
            .expect("removable spawned seat")
            .slot,
        "spawned.0"
    );
}

#[test]
fn effort_retire_each_removal_entry_closes_before_forgetting_and_retries_once() {
    for proven in [false, true] {
        let rig = Rig::new("close");
        let cleaned = Cell::new(false);
        assert_eq!(
            rig.remove(proven, &cleaned).expect("retirement publishes"),
            "spawned.0"
        );
        rig.assert_closed();
        assert_eq!(
            rig.events()
                .last()
                .expect("fresh ending")
                .summary
                .as_deref(),
            Some("retire")
        );
        assert!(
            rig.meta().contains(OTHER),
            "unrelated rows remain byte-identical"
        );
        assert!(!rig.meta().lines().any(|line| {
            line.split('=')
                .next()
                .is_some_and(|key| key.ends_with(".spawned.0"))
        }));
        assert_eq!(cleaned.get(), proven);
        let journal = rig.journal();
        assert!(
            rig.remove(proven, &Cell::new(false)).is_err(),
            "retired seat no longer resolves"
        );
        assert_eq!(
            rig.journal(),
            journal,
            "a repeated retirement cannot append another ending"
        );
    }
}

#[test]
fn effort_retire_journal_failure_keeps_all_rows_and_refuses_cleanup() {
    for proven in [false, true] {
        let rig = Rig::new("appendfail");
        let before = rig.meta();
        let journal = rig.dir().join("events.jsonl");
        std::fs::remove_file(&journal).expect("replace journal shape");
        std::fs::create_dir(&journal).expect("append refusal");
        let cleaned = Cell::new(false);
        assert!(
            rig.remove(proven, &cleaned).is_err(),
            "journal refusal must refuse forgetting"
        );
        assert_eq!(rig.meta(), before);
        assert!(
            !cleaned.get(),
            "failed publication cannot clean launch artifacts"
        );
        std::fs::remove_dir(&journal).expect("restore journal");
        std::fs::write(journal, trace()).expect("original history");
        rig.remove(proven, &cleaned).expect("retry publishes");
        rig.assert_closed();
    }
}

#[test]
fn effort_retire_reuses_an_already_committed_other_path_ending() {
    for proven in [false, true] {
        for reason in ["off", "fresh", "reseat"] {
            let rig = Rig::new("alreadyclosed");
            let journal = format!(
                "{}{{\"ts\":\"2023-11-14T22:13:21Z\",\"actor\":\"watchdog\",\"action\":\"effort-discarded\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session\":\"fixture\",\"target_session_id\":\"{SID}\",\"ref\":\"{EP}\",\"summary\":\"{reason}\"}}\n",
                trace()
            );
            std::fs::write(rig.dir().join("events.jsonl"), &journal)
                .expect("ending committed before metadata publish failed");
            rig.assert_closed();
            rig.remove(proven, &Cell::new(false))
                .expect("retirement reuses committed ending");
            assert_eq!(
                rig.journal(),
                journal,
                "an applied adjustment already has its one ending"
            );
        }
    }
}

#[test]
fn effort_retire_publish_failure_keeps_claim_and_retry_reuses_the_committed_ending() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    struct RestoreMode(PathBuf);
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755))
                .expect("restore owned fixture permissions");
        }
    }

    for proven in [false, true] {
        let rig = Rig::new("publishfail");
        let dir = rig.dir();
        let before = rig.meta();
        let uid = std::fs::metadata(dir.join("meta"))
            .expect("fixture uid")
            .uid();
        for name in ["meta.lock", "events.jsonl.lock"] {
            std::fs::write(dir.join(name), "").expect("pre-existing writable lock");
        }
        let restore = RestoreMode(dir.clone());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
            .expect("deny temp creation and rename");
        let probe = std::fs::write(dir.join("permission-probe"), "probe");
        if uid == 0 {
            assert!(
                probe.is_ok(),
                "root proves the permission fault is ineffective"
            );
            drop(restore);
            continue;
        }
        assert!(
            matches!(probe, Err(ref why) if why.kind() == std::io::ErrorKind::PermissionDenied),
            "the permission fault applies: {probe:?}"
        );
        let cleaned = Cell::new(false);
        let failed = rig.remove(proven, &cleaned);
        drop(restore);
        assert!(
            failed.is_err(),
            "metadata publication must fail after the append"
        );
        assert_eq!(
            rig.meta(),
            before,
            "failed publication retains the claim and roster"
        );
        assert!(!cleaned.get());
        rig.assert_closed();
        let journal = rig.journal();
        rig.remove(proven, &cleaned)
            .expect("retry after permissions return");
        assert_eq!(
            rig.journal(),
            journal,
            "retry cannot duplicate the committed ending"
        );
        rig.assert_closed();
    }
}

#[test]
fn effort_retire_stale_proof_cannot_end_a_successor_or_clean_its_artifacts() {
    let rig = Rig::new("successor");
    let proof = ae::identity::prove_removable(&rig.dir(), "scout").expect("old incarnation");
    let successor = rig.meta().replace("old-token", "new-token");
    std::fs::write(rig.dir().join("meta"), &successor).expect("successor owns same slot");
    let journal = rig.journal();
    let cleaned = Cell::new(false);
    let result =
        ae::identity::remove_proven_slot(&rig.dir(), "scout", &proof, || cleaned.set(true));
    assert!(matches!(
        result,
        Err(ae::identity::RemovalRefusal::Replaced { .. })
    ));
    assert_eq!(rig.meta(), successor);
    assert_eq!(rig.journal(), journal);
    assert!(!cleaned.get());
}

#[test]
fn effort_retire_damaged_claims_end_once_with_an_empty_reference() {
    for proven in [false, true] {
        for rows in [
            "effort.spawned.0\n".to_owned(),
            "effort.spawned.0=damaged\r\n".to_owned(),
            format!("effort.spawned.0={VALID}\neffort.spawned.0={VALID}\n"),
        ] {
            let rig = Rig::new("damaged");
            let metadata: String = rig
                .meta()
                .lines()
                .filter(|line| !line.starts_with("effort.spawned.0="))
                .fold(String::new(), |mut text, line| {
                    text.push_str(line);
                    text.push('\n');
                    text
                });
            std::fs::write(rig.dir().join("meta"), format!("{metadata}{rows}"))
                .expect("damaged held claim");
            rig.remove(proven, &Cell::new(false))
                .expect("damaged claim retirement");
            rig.assert_closed_ref("");
            assert_eq!(
                rig.events()
                    .last()
                    .expect("retire ending")
                    .summary
                    .as_deref(),
                Some("retire")
            );
        }
    }
}

#[test]
fn effort_retire_pending_and_restarting_claims_keep_their_episode_reference() {
    for proven in [false, true] {
        for state in ["pending-down", "restart-down", "pending-up", "restart-up"] {
            let rig = Rig::new("inflight");
            let metadata = rig.meta().replace("v1 applied", &format!("v1 {state}"));
            std::fs::write(rig.dir().join("meta"), metadata).expect("in-flight claim");
            let journal = if state.ends_with("down") {
                trace()
                    .lines()
                    .filter(|line| !line.contains("\"action\":\"effort-applied\""))
                    .fold(String::new(), |mut text, line| {
                        text.push_str(line);
                        text.push('\n');
                        text
                    })
            } else {
                format!(
                    "{}{{\"ts\":\"2023-11-14T22:13:21Z\",\"actor\":\"watchdog\",\"action\":\"effort-reserved\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session\":\"fixture\",\"target_session_id\":\"{SID}\",\"ref\":\"{EP}-up\",\"summary\":\"high->xhigh\"}}\n",
                    trace()
                )
            };
            std::fs::write(rig.dir().join("events.jsonl"), journal)
                .expect("in-flight journal evidence");
            assert!(
                matches!(
                    ae::effort::stored(rig.meta().as_bytes(), "spawned.0"),
                    ae::effort::Stored::Present(_)
                ),
                "literal in-flight record is valid: {state}"
            );
            assert!(
                matches!(
                    ae::effort::fold(
                        &rig.events(),
                        SessionKey::new("fixture", SID),
                        "spawned.0",
                        "scout"
                    ),
                    ae::effort::Fold::Open(_)
                ),
                "literal history reaches an open episode: {state}"
            );
            rig.remove(proven, &Cell::new(false))
                .expect("in-flight retirement");
            rig.assert_closed();
            assert_eq!(
                rig.events()
                    .last()
                    .expect("retire ending")
                    .summary
                    .as_deref(),
                Some("retire")
            );
        }
    }
}

#[test]
fn effort_retire_without_a_claim_leaves_the_journal_byte_identical() {
    for proven in [false, true] {
        let rig = Rig::new("noclaim");
        let text: String = rig
            .meta()
            .lines()
            .filter(|line| !line.starts_with("effort.spawned.0="))
            .fold(String::new(), |mut text, line| {
                text.push_str(line);
                text.push('\n');
                text
            });
        std::fs::write(rig.dir().join("meta"), text).expect("ordinary spawned seat");
        let journal = rig.journal();
        rig.remove(proven, &Cell::new(false))
            .expect("ordinary removal");
        assert_eq!(rig.journal(), journal);
    }
}
