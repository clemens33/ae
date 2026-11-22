//! S1b owner waits through a written board and product-opened console asks.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::console::{Console, lane, submit::Outcome, term};
use crate::events::{Event, SessionKey};
use crate::inventory::ServerId;
use crate::json::Value;
use crate::meta::Selector;
use crate::time::Timestamp;
use crate::tracked::{self, IdentityTriple, Resolved};
use crate::web::topics::{self, Column, Sticky};

const UUID: &str = "0199c0de-9876-4890-abcd-ef0123456789";
const OLD_NAME: &str = "former-review";
const NEW_NAME: &str = "renamed-review";
const OPENED: i64 = 1_792_141_200;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        let temp = std::env::temp_dir();
        loop {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let root = temp.join(format!("ae-wtpc-{}-{sequence}", std::process::id()));
            match std::fs::create_dir(&root) {
                Ok(()) => return Self(root),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create scratch {}: {error}", root.display()),
            }
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::write(path, bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

struct Ledger {
    scratch: Scratch,
    session: &'static str,
    dir: PathBuf,
}

impl Ledger {
    fn new() -> Self {
        let scratch = Scratch::new();
        let dir = scratch.0.join("sessions").join(OLD_NAME);
        std::fs::create_dir_all(dir.join("messages"))
            .unwrap_or_else(|error| panic!("create session: {error}"));
        let fixture = Self {
            scratch,
            session: OLD_NAME,
            dir,
        };
        write(
            &fixture.root().join("orchestrator.config"),
            "[workspace]\norchestrator = true\n",
        );
        fixture.meta();
        fixture
    }

    fn root(&self) -> &Path {
        &self.scratch.0
    }

    fn meta(&self) {
        write(
            &self.dir.join("meta"),
            format!(
                "schema=2\nsession={}\nsession_id={UUID}\nlayout=lead-pair\n\
                 seat.main=lead\nseat.worker.0=colead\nlaunch_id.main=fixture-launch\n\
                 tmux_server_kind=socket\ntmux_server={}\nmeta_agent=true\nlocal_config={}\n",
                self.session,
                self.root().join("pane-socket").display(),
                self.root().join("orchestrator.config").display(),
            ),
        );
    }

    fn rename(&mut self) {
        let next = self.root().join("sessions").join(NEW_NAME);
        std::fs::rename(&self.dir, &next)
            .unwrap_or_else(|error| panic!("rename fixture session: {error}"));
        self.dir = next;
        self.session = NEW_NAME;
        self.meta();
    }

    fn store(&self) -> crate::store::SessionStore {
        crate::store::open(&self.dir)
    }

    fn append(&self, fields: &[(&str, &str)]) {
        let event = Value::obj(fields.iter().map(|(key, value)| (*key, Value::str(*value))));
        self.store()
            .append_event(&format!("{}\n", event.render()))
            .unwrap_or_else(|error| panic!("append fixture event: {error}"));
    }

    fn publish_board(dir: &Path, text: &str) {
        let items = crate::desk::parse_input(text.as_bytes())
            .unwrap_or_else(|error| panic!("valid desk fixture: {error}"));
        crate::deskstore::set(dir, items, Timestamp::from_epoch(OPENED), "lead")
            .unwrap_or_else(|error| panic!("publish board fixture: {error}"));
    }

    fn owner_ask(&self, entropy: u64, body: &str) -> String {
        let pair = ["lead".to_owned(), "colead".to_owned()];
        let socket = self.root().join("pane-socket");
        let now = Timestamp::from_epoch(OPENED);
        let kind = tracked::Kind::Ask;
        let request = tracked::request_id(kind.id_prefix(), now, entropy);
        let caller = tracked::CONSOLE_DISPLAY;
        let composed = tracked::compose(kind, &request, caller, body, "./reply fixture");
        let frame = crate::provenance::first_line(&crate::provenance::peer(caller), &composed);
        let stored = self
            .dir
            .join("messages")
            .join(format!("{request}.ask.0000aa.txt"));
        write(&stored, &frame);
        tracked::clear_test_hooks();
        tracked::set_test_resolve(
            Resolved {
                pane: "%4".to_owned(),
                agent: "lead".to_owned(),
                slot: "main".to_owned(),
                session: self.session.to_owned(),
            },
            ServerId::Selected(Selector::Socket(socket.clone())),
        );
        let checked = IdentityTriple {
            server: socket.to_string_lossy().into_owned(),
            pane: "%4".to_owned(),
            session_uuid: UUID.to_owned(),
        };
        tracked::queue_observe(Ok(checked.clone()));
        tracked::queue_observe(Ok(checked));
        tracked::set_test_delivery(Ok(crate::deliver::Delivered {
            body_file: stored.to_string_lossy().into_owned(),
            framed: frame,
            verification: crate::deliver::DeliveryVerification::Verified,
        }));
        let carried = term::submit_external(
            self.root(),
            term::External {
                dir: &self.dir,
                session: self.session,
                session_id: UUID,
                pair: &pair,
                seat: Some("lead"),
                body: body.to_owned(),
                minted: (now, entropy),
            },
        )
        .unwrap_or_else(|error| panic!("a valid console ask: {error}"));
        tracked::clear_test_hooks();
        assert!(!carried.replayed);
        assert_eq!(carried.id, request);
        assert!(matches!(&carried.outcome, Outcome::Sent(id, _) if id == &carried.id));
        carried.id
    }

    fn reply(&self, id: &str) {
        let socket = self.root().join("pane-socket");
        self.append(&[
            ("ts", "2026-10-16T09:01:00Z"),
            ("actor", "lead"),
            ("action", "reply"),
            ("target", "console:local"),
            ("ref", id),
            ("actor_slot", "main"),
            ("actor_session", self.session),
            ("caller_server", socket.to_str().unwrap_or_default()),
            ("caller_pane", "%4"),
            ("caller_session_uuid", UUID),
            ("summary", "The corresponding reply"),
        ]);
    }

    fn cancel(&self, id: &str) {
        let console = Console::bound(self.session.to_owned(), self.dir.clone(), UUID.to_owned());
        let lock = self
            .store()
            .console_admission_now()
            .unwrap_or_else(|error| panic!("fixture close admission: {error}"));
        let _ = console
            .close_owned(Some(id), || Ok(()), lock)
            .unwrap_or_else(|error| panic!("close eligible console ask: {error}"));
    }

    fn retire_main(&self) {
        self.append(&[
            ("ts", "2026-10-16T09:02:00Z"),
            ("actor", "lead"),
            ("action", "retire"),
            ("target", "lead"),
            ("target_slot", "main"),
            ("target_session", self.session),
            ("target_session_id", UUID),
        ]);
    }

    fn records(&self) -> Vec<Event> {
        let bytes = self.store().container();
        let text =
            std::str::from_utf8(&bytes).unwrap_or_else(|error| panic!("journal UTF-8: {error}"));
        text.lines()
            .map(|line| {
                Event::parse_line(line).unwrap_or_else(|error| panic!("fixture event: {error:?}"))
            })
            .collect()
    }
}

fn wait_ids<'a>(column: &'a Column, slug: &str) -> Vec<&'a str> {
    column
        .waits
        .get(slug)
        .into_iter()
        .flatten()
        .filter_map(|wait| wait.id.as_deref())
        .collect()
}

#[test]
fn tagged_decide_items_join_only_the_orchestrator_board() {
    let fixture = Ledger::new();
    Ledger::publish_board(
        &fixture.dir,
        "DECIDE\nD1 | Choose now #release, then publish\nD2 | an untagged decision\n\
         COMING\nC1 | a coming item #coming\nRUNNING\nR1 | an active item #running\n",
    );
    let ordinary = fixture.root().join("sessions/ordinary");
    std::fs::create_dir(&ordinary).unwrap_or_else(|error| panic!("ordinary session: {error}"));
    write(
        &ordinary.join("meta"),
        "schema=2\nsession=ordinary\nsession_id=0199c0de-9876-4890-abcd-ef0123456781\nlayout=lead-pair\nseat.main=other\n",
    );
    Ledger::publish_board(&ordinary, "DECIDE\nD1 | another session waits #other\n");

    let column = topics::collect(fixture.root(), &mut Sticky::default());
    let tagged = column
        .waits
        .get("release")
        .unwrap_or_else(|| panic!("tagged decision"));
    assert_eq!(tagged.len(), 1);
    assert_eq!(tagged[0].text, "Choose now #release, then publish");
    assert_eq!(tagged[0].id, None);
    assert_eq!(column.untagged.len(), 1);
    assert_eq!(column.untagged[0].text, "an untagged decision");
    assert_eq!(column.untagged[0].id, None);
    for name in ["coming", "running", "other"] {
        assert!(
            !column.waits.contains_key(name),
            "non-DECIDE/role source: {name}"
        );
    }
    assert!(column.groups.iter().any(|(group, cards)| {
        matches!(group, crate::topic::Group::NeedsYou)
            && cards.iter().any(|card| card.slug == "release")
    }));
}

#[test]
fn real_console_opens_stop_owing_after_reply_cancel_and_uuid_bound_retirement() {
    let mut fixture = Ledger::new();
    let body = format!(
        "{}\n#full-body, held past the opening preview",
        "x".repeat(650)
    );
    let answered = fixture.owner_ask(71, &body);
    let cancelled = fixture.owner_ask(72, "#summary-fallback can survive a missing body");
    let retired = fixture.owner_ask(73, "#renamed-ask is awaiting a reply");
    let missing = fixture
        .dir
        .join("messages")
        .join(format!("{cancelled}.ask.0000aa.txt"));
    std::fs::remove_file(missing)
        .unwrap_or_else(|error| panic!("remove owned fixture body: {error}"));

    let event_log = fixture.records();
    let opening = event_log
        .iter()
        .find(|event| event.reference.as_deref() == Some(answered.as_str()))
        .unwrap_or_else(|| panic!("real opening recorded by submit_external"));
    let summary = opening
        .summary
        .as_deref()
        .unwrap_or_else(|| panic!("opening preview"));
    assert!(!summary.contains("#full-body"));
    assert_eq!(
        crate::console::ask_body(&fixture.dir, opening),
        lane::Body::Whole(body)
    );

    let mut sticky = Sticky::default();
    let opened = topics::collect(fixture.root(), &mut sticky);
    assert_eq!(wait_ids(&opened, "full-body"), [answered.as_str()]);
    assert_eq!(wait_ids(&opened, "summary-fallback"), [cancelled.as_str()]);
    assert_eq!(wait_ids(&opened, "renamed-ask"), [retired.as_str()]);

    fixture.reply(&answered);
    fixture.cancel(&cancelled);
    let reduced = topics::collect(fixture.root(), &mut sticky);
    assert!(!reduced.waits.contains_key("full-body"));
    assert!(!reduced.waits.contains_key("summary-fallback"));
    assert_eq!(wait_ids(&reduced, "renamed-ask"), [retired.as_str()]);

    fixture.rename();
    let renamed = topics::collect(fixture.root(), &mut sticky);
    assert_eq!(wait_ids(&renamed, "renamed-ask"), [retired.as_str()]);
    fixture.retire_main();
    let event_log = fixture.records();
    assert!(lane::open_asks(&event_log, NEW_NAME).contains(&retired.as_str()));
    let ledger = fixture.store().container();
    let key = SessionKey::new(NEW_NAME, UUID);
    let record = crate::requests::states_in(&ledger, key)
        .into_iter()
        .find(|record| record.id.as_slice() == retired.as_bytes())
        .unwrap_or_else(|| panic!("request still readable after retirement"));
    assert!(matches!(
        record.lifetime_in(&ledger, key),
        crate::requests::Lifetime::Retired(_)
    ));
    let final_column = topics::collect(fixture.root(), &mut sticky);
    assert!(!final_column.waits.contains_key("renamed-ask"));
}

#[test]
fn other_actors_and_a_console_actor_with_a_relay_stamp_do_not_create_owner_waits() {
    let fixture = Ledger::new();
    let control = fixture.owner_ask(41, "#owned this is an owner ask");
    for (id, actor, target, summary) in [
        (
            "ae-20261016T090100Z-00000042",
            "worker",
            "lead",
            "#inter-agent another seat asks",
        ),
        (
            "ae-20261016T090100Z-00000043",
            "lead",
            "telegram:owner",
            "#telegram bridge-addressed opening",
        ),
        (
            "ae-20261016T090100Z-00000044",
            "lead",
            "discord:owner",
            "#discord bridge-addressed opening",
        ),
    ] {
        fixture.append(&[
            ("ts", "2026-10-16T09:01:00Z"),
            ("actor", actor),
            ("action", "ask"),
            ("target", target),
            ("target_slot", "main"),
            ("target_session", OLD_NAME),
            ("target_session_id", UUID),
            ("ref", id),
            ("summary", summary),
        ]);
    }
    let relayed = "ae-20261016T090100Z-00000045";
    fixture.append(&[
        ("ts", "2026-10-16T09:01:00Z"),
        ("actor", "console:local"),
        ("action", "ask"),
        ("target", "lead"),
        ("target_slot", "main"),
        ("target_session", OLD_NAME),
        ("target_session_id", UUID),
        ("ref", relayed),
        ("summary", "#dispatcher a relay is not the owner"),
        ("relay_origin", "0199c0de-9876-4890-abcd-ef0123456799"),
        ("relay_mode", "attested"),
    ]);
    let events = fixture.records();
    assert!(lane::open_asks(&events, OLD_NAME).contains(&relayed));

    let column = topics::collect(fixture.root(), &mut Sticky::default());
    assert_eq!(wait_ids(&column, "owned"), [control.as_str()]);
    for slug in ["inter-agent", "telegram", "discord", "dispatcher"] {
        assert!(
            !column.waits.contains_key(slug),
            "a non-owner opening created wait {slug}"
        );
    }
}
